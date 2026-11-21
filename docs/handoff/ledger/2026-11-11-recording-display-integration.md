# Recording Display integration — targeted host controls passed

Display now owns a recording session state machine above the dedicated atomic64
bank. Actual stamped sender/main-task identity, the held Shell route epoch, and
matching held recorder/Spindle lifecycle catalogs bracket admission and copy.
Stop and expiry revoke the session and scrub before retaining any response or
closure event. A closed session remains a replay tombstone. App Release cannot
register unknown scope; registered pending-Arm Release revokes and supersedes an
older Ready/Cancelled receipt. Frame ACK requires the exact delivered generation;
a newer Acquire clears previous ACK replay eligibility.

An immutable Scene records the accepted scope descriptively. Its protected Chip0
stop ring/square is painted after ordinary overlays/cursor, independent of network
chip kind/visibility. Full/strip CPU publication must succeed and its completed
private front must contain every expected marker pixel before Armed is retained.
Cursor-only/settle publication never establishes Armed. Scene mark, current held
scope and liveness are rechecked before a due Acquire copies the completed front
into the one-MiB bank. No front pointer escapes or persists. Output is fixed
640x360 letterboxed, no faster than 200 ms per frame, for at most 60 seconds.
Publication timestamps come from the actual successful CPU publication turn.

Retained positive responses are revalidated before delivery. Shell owns the
trusted Chip0 click and keyboard Stop input routes; Display grants no app arm
route. Existing screenshot capture remains separate. BOOT and app/Files wiring
are owned by the recorder integration branch, not this change.

New targeted host controls compile the actual Display server, bank, marker and
extracted cursor/save-under functions with explicitly synthetic syscall/backing
fixtures. They cover sender/scope/visible-publication checks, backpressure,
delayed ACK against a later frame, Stop/release tombstones, expiry/retirement,
network changes, clipped placement, and marker pixels at scales 1–3. Command
`python3 -m unittest scripts.tests.test_display_recording_capture scripts.tests.test_recording_indicator_pixels`
passed both suites: seven server/bank controls and one extracted cursor/marker
pixel control, in 1.294 seconds. Compilation was serialized with the JIT worker.
No native, PKU/read-fault, encoded timestamp, real click/keyboard Stop or
working-recorder claim is made. Matching CPU-front/native evidence and the integrated vertical
slice remain required. No kernel growth or new compatibility mechanism.
