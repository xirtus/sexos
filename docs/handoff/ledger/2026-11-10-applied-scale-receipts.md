# Applied scale receipts before shared chrome migration (2026-11-10)

The approved slim UX implementation continues after the shared-metrics slice.
A successful Shell PDX call only enqueues a step; it cannot attest that Display
painted it. Shell now retains one sequenced request and commits its applied-scale
state only on a matching reply from the trusted Display domain. Extra scale keys
while one request is outstanding are consumed without enqueuing another step.
Unrelated input remains live. BOOT owns reset scale; Display restart cancels pending
Shell state and reloads that BOOT value.

Display retains one pending receipt and requires a complete CPU frame publication
before replying. Failed/partial output cannot enable it; reply backpressure retains
it. Sequence zero keeps the existing fire-and-forget convention, while sequenced
requests use the previously zero arg1. Malformed sequence/scale values and a second
outstanding request cannot replace the retained publication state. No new opcode,
capability or kernel API is introduced. A CPU publication receipt does not attest
hardware scanout or a monitor latch.

Independent production-SDK cases cover admission, retained transport failure,
publication gating, rejected callers/tags, stale replay, malformed requests and
restart cancellation. The native-resolution gate now requires the matching Display
and Shell receipts after a completed frame as well as its existing pixel-ink scale
1->2->1 and bounded-damage checks. Shell's live chrome geometry is still unchanged;
the next slice wires the shared 28/26 metrics through paint, hit/input and clients.

Host validation: six Rust cases and the Python wrapper pass; 31 focused Python
cases pass; ABI pins regenerated. Clean target build/native guest qualification
is required before acceptance. Kernel growth zero; retained states are fixed-size.

Review correction: all new scale changes, including sequence-zero legacy calls,
are rejected while a publication receipt is pending. Shell service restarts rearm
the same task id (kernel service_restart.rs), so caller_task alone cannot fence an
old reply. Shell seeds disjoint sequence blocks from its kernel-owned own-service
restart count. An authenticated new WM registration carries that count; Display
checks the exact coordinator main task, rejects epoch rollback, clears an old run's
pending receipt only on a newer registration, and resets scale to BOOT for that run.
Idempotent registration cannot drop an outstanding reply. Sequence exhaustion fails
closed instead of wrapping. Production-state controls cover legacy interference,
old reply replay into a new Shell, task mismatch and registration rollback.
