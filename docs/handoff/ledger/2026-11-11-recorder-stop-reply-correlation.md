# Stop intent reply correlation

Spindle advances its person-intent sequence for record-stop. Shell previously revoked capture but retained the original sequence for Stopped/Saved notifications, so the sender could ignore actual completion. For a newer Stop from the same actual current Spindle task, Shell now preserves the immutable scope owner/session but moves status routing to the latest intent sequence. Stale Stop still revokes only and cannot roll the status sequence backward.

One focused production-controller test passes: Stop preserves the exact Arm scope, Stopped uses sequence2, and a stale sequence1 Stop cannot redirect the later Saved notification. Compile/test logs retained under desktop-recorder/stop-correlation-*.log. This fixes status routing, not a capture authority check; native recorder completion remains pending the separate BOOT reservation-capacity fix.
