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

Refreshed `9409b3e7` clean build and 2560x1600 native receipt gate passed. The
1280x800 run correctly rejected the diagnostic ordering: pdx_reply immediately
scheduled Shell, so Shell's applied marker could precede Display's post-reply
publication log even though the completed full-frame log preceded both. Publication
is now logged once after that full frame and before transport, with a separate sent
marker only after successful reply. The strict gate assertion is retained unchanged;
failed evidence remains under local chrome-scale-receipts/fenced9409. Seven production
state cases pass. Fresh final-source native retries and current-head CI are required.

Final publication-marker source `cc9d276c` rebuilt and passed both strict native
2560x1600 and 1280x800 scale 1->2->1 gates (minimum updates 32/37 pixels). A custody
source guard then required preserving the established workspace/lifecycle poll
prefix; scale polling follows it without changing immediate key-path admission.
The extracted shortcut fixture now includes the real ScaleClient and production
poll helper over its declared syscall double, with a new exact-request backpressure
and duplicate-step control. Full clipboard and service-custody host gates pass.
Current-head CI and a final source-bound native refresh remain required after this
poll-order change; earlier failed runs are retained, not silently replaced.
