# Spreadeagle power-cut recovery proof

- Extended the Programs QEMU gate to edit a saved workbook without Ctrl+S, wait for Spreadeagle's idle recovery receipt, capture the unsaved grid, and kill QEMU while the app remains open.
- The gate boots the same disk, checks the recovery receipt, the edited cell and dependent formula values, captures the restored grid, then saves and reopens the workbook to confirm recovery is committed.
- This proves the guest recovery path under a QEMU power cut after the recovery write completes. It does not claim a physical drive's flush behavior during a power failure.
- Validation: Python syntax and `git diff --check` locally. Full image build and QEMU execution are on the branch's GitHub CI run; the `sexos-programs-proof` artifact supplies `spreadeagle-unsaved.ppm` and `spreadeagle-recovered.ppm` for the PR screenshot.
