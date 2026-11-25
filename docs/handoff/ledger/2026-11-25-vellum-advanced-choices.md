# Vellum advanced PDF choices

- Added keyboard editing for AcroForm editable combo boxes and multi-select list boxes. Editable combos accept typed custom text or paired option exports; multi-select lists visibly toggle each option and store the selected export values. Read-only widgets stay inaccessible.
- Added a real PDF fixture with custom combo text and a two-value list. Host editor tests check the PDF after incremental save/reopen; the Vellum viewer test checks focus and selection helpers. The Documents QEMU gate types a custom value, changes a list selection, captures `pdf-advanced-choice-fields.ppm`, saves through the confirmed Files sync path, reopens, captures `pdf-advanced-choice-reopened.ppm`, and extracts the guest-written PDF for an independent host value check.
- Pointer controls and richer combo text editing remain. Full image build and QEMU run belong to GitHub CI.
