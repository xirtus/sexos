# Exact launch-ticket ownership

Files now binds every program-prepared launch ticket to the preparing protection
domain and its exact live main task. Shell forwards that parent identity when
loading generic tickets and rejects stale stamped main-task launch/control
requests. Console relay attachment remains separate from ticket authorization.
Trusted Spindle tickets retain requester zero. Tickets remain single use.

The original compiled production-method counterexample is preserved under
`target/legacy-ticket-counterexample` and in
`/tmp/sexos-legacy-ticket-counterexample.log`: both row reuse and another parent's
generic ticket were accepted before the fix. This is host reproduction evidence,
not a guest exploitation claim.

Validation: two Python tests compile and exercise the actual authorization methods
and check their production wiring. The recording host gate passes 51 Rust tests
and seven Python checks; its typed route forwards the admitted caller task.
Canonical Files and Shell PIC/PIE release builds pass. Independent peer review
found no blocking issue. Logs are `/tmp/sexos-prepared-owner-tests-recording-final.log`,
`/tmp/sexos-ticket-recording-host-v2.log`,
`/tmp/sexos-prepared-owner-files-build-v2.log`, and
`/tmp/sexos-ticket-shell-build.log`. Initial build/fixture failures remain intact.

Guest routing qualification on a new combined image is pending. Ordinary worker
file I/O retains its existing behavior; this fix does not claim general worker
membership or stale-worker I/O isolation. No kernel changes are included.
