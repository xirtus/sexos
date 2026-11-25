# Vellum annotation deletion undo

- Deleting an annotation in the list now records its exact PDF page-array entry and popup, allowing Ctrl+Z to restore its original object and appearance and Ctrl+Shift+Z to remove it again. The history remains bounded to 32 changes and is cleared on save.
- The editing engine refuses an ambiguous redo match, so it cannot remove a different direct annotation. Focused engine and viewer-history tests cover saved PDF round trips. The Documents QEMU gate deletes a highlight, undoes it, captures `pdf-deletion-undone.ppm`, then redoes the deletion before its existing save assertions.
- Validation: focused host tests and Python syntax check locally; full image build and QEMU gate only in GitHub CI. No target or disk image is kept locally.
