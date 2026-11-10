# Shared QEMU boot setup cleanup

The shared `quil_autosave_gate.boot()` previously launched QEMU before its cleanup
closure was available, then waited for its QMP socket, handshake and boot receipts.
A failure or interrupt during that setup left the guest running without a caller
able to stop it. Define cleanup before setup and invoke it on setup exceptions,
including interrupts. Kill/reap a still-running guest and unlink its QMP socket;
retain serial and QEMU diagnostic files. Successful return still transfers cleanup
to the caller. Close the parent's QEMU stdout handle after process launch.

Five mocked-process regressions exercise socket timeout, QMP handshake failure,
missing readiness receipt, interrupt, and successful ownership transfer with
idempotent stop. All pass with ResourceWarning treated as an error. Shared serial
receipt tests also pass. No VM or OS source changes needed for this lifecycle fix.
