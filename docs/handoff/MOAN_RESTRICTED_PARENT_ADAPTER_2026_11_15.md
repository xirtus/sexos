# Restricted parent adapter checkpoint

Moan's existing `OfflineJob::start_with` now delegates to
`start_restricted_with`. The parent uses immutable Files admission metadata and
the restricted Shell coordinator; there is no ordinary launch-ticket fallback.
The initial input digest must match the parent's encoded request. Publication
requires the full nonce, hashes, parent, cookie, format, frame count, provider and
coordinator epochs, retained authenticated child, and physical output SHA-256.

The SDK brackets staging, admission, metadata and output file reads with the held
Files epoch. Requests enqueue exactly once after queue admission. A pending
request owns the Files turn; after an uncertain same-epoch exit, the SDK refuses
region and stat-cache reuse until a verified new live Files epoch. This narrows
SDK behavior, not arbitrary raw-syscall clients. Each synchronous file RPC is
bounded; staging and result copying remain synchronous, with no frame-time claim.

Cancellation and metadata release use a nonblocking SDK End state. Poll never
waits for its reply. Moan retains the job and disables file/project actions until
actual child retirement plus either cleanup ACK or verified Files replacement. Errors,
timeouts and unknown status cannot declare completion. An accepted result is
retained through release; a rejected result retains Failed through cleanup.

Shell restart recovery only accepts an exact lifecycle observation for a child
known from authenticated Started, correlated by cookie, ticket and current Shell
epoch. No Started/unknown dormant state stays unavailable/pending; it is never
relabeled retired. Same-epoch poisoned cleanup can require service restart.

Validation: 29 host tests pass (14 existing plugin tests, eight tests compiling
the actual SDK RPC, seven compiling the actual parent supervisor). They cover
delayed replies without duplicate admission, queue backpressure, unavailable or
changed epochs, timeout/abandonment, bounded staging/read, result substitution,
cleanup backpressure/error, stale child and unknown recovery. Actual Moan guest
target checking also passes. Logs:

- `/tmp/sexos-root-restricted-supervisor-all-host-final.log`
- `/tmp/sexos-root-restricted-rpc-raw-fs-final.log`
- `/tmp/sexos-root-restricted-moan-controller-guest-final.log`

Initial compiler/setup failures remain intact. Independent source review found
and corrected post-enqueue poison, duplicate request, cleanup-error and rejected
result lifetime issues before this checkpoint. Runtime crash/restart and isolation
qualification is incomplete: automatic security review stopped the test agent,
including the reduced crash/restart scope. No new QEMU sandbox proof, hostile-code
containment, installer, merge or current-master compatibility claim is made.

Follow-up UI review keeps every busy Close deferred until cleanup, including a
completed result awaiting memory retry, so Discard is not opened twice. A
Finishing state keeps Play disabled after automatic result import until cleanup
ends. The updated real allocation-failure fixture still injects failures in both
result-byte allocation and quantization, verifies no retry spin, retries the same
completed job once and retains it through asynchronous release. Moan's 60 host
tests, including the finishing-state project/Play lock, pass in
`/tmp/sexos-root-moan-cleanup-ui-host-v2.log`; the updated OOM/Term focused set
passes 13 tests in `/tmp/sexos-root-repaired-fixtures-focused.log`.
