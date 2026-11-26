# Loom final copy sync

Background jobs now report completion only after the final Files sync succeeds.
An injected sync failure exercises rollback policy in `loom-core`: a copy can
report confirmed cleanup only when destination removals and a cleanup sync both
succeed. Otherwise Loom says storage state is unconfirmed and asks the person
to inspect source and destination. A move already deleting its source and a
partly completed permanent delete cannot claim rollback.

The Loom QEMU gate checks the successful final-sync log after batch readback,
captures `loom-sync-confirmed.ppm`, and retains its content and undo checks.
Full build and QEMU run only on GitHub CI. This does not implement cross-disk
batch cut or power-cut recovery.
