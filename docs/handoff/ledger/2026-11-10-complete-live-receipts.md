# Complete live serial receipts

`quil_autosave_gate.boot()` now searches only newline-terminated serial text in
its shared `wait()` helper. It previously searched the trailing UART write while
it was still growing, allowing capture groups to accept prefixes of values.
Raw `text()` and health scanning still include that tail; firmware's unterminated
Limine compact-mode menu uses the sole explicit `raw=True` wait.

PR #133's one-core JIT CI run 42827310405, job 129867889325, failed after capturing
`124929280|2147483648|-2147`. Its untouched artifact 13769798905 contains the final
complete, correct result `124929280|2147483648|-2147483649|4294967296|true|a1|2`.
Raw browser serial: 1,092,140 bytes, SHA256
`19b2f462da6849f7dae6efc50a6bf669fc2f7bfea64401c6ca7841554f1ffdc9`.
Evidence retained at `/Users/xmacbookm5/Projects/sexos-evidence/serial-receipts/pr133/`.
This also protects numeric timing receipts (for example media's `moved after`)
from accepting only their first digits. Complete malformed records remain visible
to the caller's existing validation; no arbitrary final record is discarded.

Validation: nine host regressions cover actual failed JIT prefix, bytewise
LF/CRLF delivery, task/counter/timing captures, offsets, complete malformed
records, explicit raw firmware mode, and partial panic/fault/overflow markers.
Audited all 73 direct importers of shared boot: their wait receipts are
newline-producing structured records or console lines. No OS source change.
Full Python discovery: 1,156 tests passed; whitespace checks passed.
