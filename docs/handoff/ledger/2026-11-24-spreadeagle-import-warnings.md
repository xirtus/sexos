# Spreadeagle import warnings in the workbook

- Branch: `codex/spreadeagle-import-warnings`, stacked on
  `codex/spreadeagle-compound-filters` (#658).
- `Book::open` warnings now reach the native workbook UI. The panel opens on
  import, reviews each warning with Up/Down, closes with Enter/Esc, and reopens
  with Ctrl+Shift+W. Engine branding is replaced with Spreadeagle in the UI.
- `import-warning.ods` has an amber source cell fill that the ODS data-only
  importer drops; the focused host test confirms the genuine warning.
- The Programs QEMU gate fetches that fixture into the guest, opens it, checks
  the warning and panel records, captures `spreadeagle-import-warning.ppm`, and
  verifies dismissal and reopening. Full builds and QEMU run on GitHub CI only.
