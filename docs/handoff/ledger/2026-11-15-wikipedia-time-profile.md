# Wikipedia elapsed-time attribution

PR274 now includes the interpreter property/string caches, at the user's request,
on 675b0ea19978ad05c612b74af6953182cddd9496. Run 43657249033 must pass every
exact-head check including aggregate build before merge. The superseded queued
run 43654690898 was force-cancelled; no other branch's run was cancelled.

The separate `codex/jit-native-object-string` branch adds an optional `jit-profile`
build feature, enabled in the separate browser builder with
`SEXOS_WEB_JIT_PROFILE=1`. Production builds have no added clocks or counters.
Both guest backends attribute host calls by operation and JIT admission/execution
separately. A deterministic-clock test exercises actual text and byte host calls
and verifies elapsed attribution against the enclosing VM duration. Native build
passes. This change does not implement native object/string lowering.

On genuine HTTPS Wikipedia's Microkernel page with the same physical scrolling
workload, cumulative VM elapsed time was 14.371 seconds. Host operations consumed
5.402 seconds, of which 402 selector queries consumed 5.113 seconds (35.6% of VM
time). JIT admission, compilation and execution together consumed 0.148 seconds
across 18,463 calls. The 8.821-second remainder includes guest execution, bridge
work and instrumentation overhead; it is not a pure JavaScript CPU measurement.
The clock has 1 ms resolution and includes preemption. This single instrumented
run does not establish a speedup or support comparisons with uninstrumented runs.

Wikipedia loaded with 81 text matches, 2,291 paint items, 778 links and document
height 22,112. Its departure profile was captured before the auxiliary webmail
step failed: the current harness expected 20 Shell palette entries while the
frozen ISO reported 14. The full site suite therefore did not pass. Do not relax
the palette assertion or relabel the partial run as an end-to-end pass. The VM
was stopped and reaped by the harness.

Evidence: `target/browser-evidence/wiki-profile/profile.json`, raw serial and
screenshots in that directory; failed wrapper output in `wiki-profile.log`.
Browser SHA256 e0292e96ca58ea814ba5099bc9145b6507778290fb180fa6455ba91626b6daa5.
Raw serial SHA256 120f4927e8b065a8237b80d4fa48c1fe79456f992be49794caa6ac069c12d383.
Frozen ISO SHA256 c6eaec57eb55cbbfd4b9bd1afc20caa8b6531d342e73ba8629ad89040443356c.

Native object/string lowering, its differential/isolation verification and a
new same-page performance comparison remain unfinished. Visible decoded YouTube
playback remains reserved for the x17.
