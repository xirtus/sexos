# Microtask checkpoint investigation — not a production fix

The merged browser fixes now load the genuine Google-hosted YouTube demo and its
embed iframe over HTTPS with actual parent Referer. Coherent source 01c3de2e
normal ISO/Web still stops at `writeEmbed is not defined`; playback is unverified.
The initial external-script load-event fix passes its independent native gate but
does not resolve that bootstrap error.

## Observed separately

The guest `run_jobs` silently stops after 10,000 `JS_ExecutePendingJob` calls.
Private instrumentation preserves that behavior and counts the pinned QuickJS
runtime's actual pending-job list. A finite 50,000-step Promise chain on the ARM
host/wasmi emitted `jobs=10000 pending=1`; the next JS evaluation observed only
20,000 steps after the previous eval/flush returned. Later execution reached
35,156 and reported `InternalError: interrupted` under the separate 10-second
flush budget. This establishes unfinished checkpoint behavior and a timing/budget
confound, not YouTube causality or native-tier throughput.

Evidence: `Projects/sexos-evidence/microtask-cap-finite/result.json`, including
probe/C/log/Wasm hashes. Private instrumentation source 7d4bd361, Wasm SHA256
`d7a860d2407196121e835bd790ce2f44130ebf94ec05f6397ff08c9f60308e83`.
The source checkout remains pinned; the counter is appended only to the build's
temporary patched QuickJS copy. No shipping queue limit or API has changed.

Earlier offline observations checked writeEmbed only after an additional eval
and ready dispatch. They do not establish which operation caused progress.
The captured parser input is SHA256
`66f3b3e4d88ff739e2c2be8d34cab719dc9b7ff97f7f4cc9a56df608e3d2ace1`;
`embed-capture-real-https/capture-provenance.json` binds its private loopback
collection to the actual guest/program/ISO. Private collection is absent from
the accepted normal image.

## Next proof

Run the finite-chain fixture on a coherent native image to remove slow ARM/wasmi
budget effects. Then measure actual cap/pending counts during the real HTTPS
load. Private branch c2e6a257 is based on accepted 01c3de2e with only counter C,
its reproducible Wasm snapshot and build-script changes. It is not a release PR.

## Requirements if the cap needs a product fix

Do not raise the limit for one site or run ready/timer events to unblock it.
A checkpoint must report drained, yielded or budget failure. Quanta need both
job-count and elapsed-time bounds; all quanta share the original invocation's
aggregate deadline. A single long job retains the existing interrupt check.

The browser must retain the owning Script/page/frame and next initial-script
position while yielded. It must complete the microtask checkpoint before the
following initial script or ready/load completion, check close/navigation between
quanta, and discard pending jobs and owned JIT code on cancellation. It must not
reenter that VM with unrelated events while its checkpoint is pending. Timers
remain separate tasks. Timeout must be an explicit failure, not partial success.

Acceptance should cover a finite chain before a following inline script, an
endless chain's bounded termination, responsive close, navigation/frame removal,
error/muting policy, code-page retirement and interpreter/native/JIT parity.
The scheduler belongs in user-space engine/browser code; no kernel policy or
Linux/POSIX surface is needed. This is a candidate design, not implemented API.
