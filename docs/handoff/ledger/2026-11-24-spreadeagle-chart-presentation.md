# Spreadeagle chart presentation

- Ctrl+Shift+Q now accepts `table on/off` for chart types GridCraft supports. The same chart command remains undoable; invalid values and unsupported chart types show an error. GridCraft renders the data table inside Spreadeagle.
- A focused host test checks validation, native preview pixels, undo/redo, and XLSX save/reopen. The Programs QEMU gate edits a live chart and captures `spreadeagle-chart-presentation.ppm`.
- Full image build and QEMU evidence run on GitHub CI. Local focused host, parser, gate syntax and diff checks are recorded in the PR.
