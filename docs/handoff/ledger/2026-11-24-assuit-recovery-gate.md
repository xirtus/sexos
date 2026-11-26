# Assuit recovery across power loss

Add a two-boot Programs QEMU gate. It edits a document without saving, waits for
Assuit's idle recovery receipt, kills the guest without closing the app, then
reboots the same disk. The gate checks that the edit is restored, captures the
restored window, saves, and reopens the document without a recovery copy.

This covers persisted recovery after a cut following the recovery receipt. It
does not claim that a cut during Files' rename has been exercised. The Assuit
save path stages and syncs its bytes before that rename.

Scope: `scripts/program_assuit_recovery_gate.py`, Programs CI, and Assuit docs.
No kernel, driver, or tether changes. Full build and QEMU evidence run in GitHub
CI; the screenshot and run link belong in the PR.
