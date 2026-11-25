# Spreadeagle multi-column sort

- Ctrl+Shift+J opens a native sort prompt for an explicitly selected table. It accepts two to eight distinct columns in priority order, each `asc` or `desc`, and leaves GridCraft's detected header at the top.
- A host test covers invalid keys, secondary ordering, undo/redo and XLSX save/reopen. The Programs QEMU gate selects a table, invokes the prompt, checks the native action log and captures `spreadeagle-multikey-sort.ppm`.
- Full image build and QEMU execution run in GitHub CI; only the focused host test and syntax checks run locally.
