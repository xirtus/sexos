# Native proof of incomplete initial-script checkpoints

Private coherent source 90e1d0ed (accepted 01c3de2e plus queue counters/docs)
reproduces the checkpoint defect under the native wasm2c tier in a four-core
QEMU guest. The first inline script queues a finite 50,000-step Promise chain;
the following inline script observes **40,000**, rather than 50,000.

Four actual C drain receipts each report 10,000 processed jobs and exactly one
pending job. There is no interruption in this run. The browser reaches ready and
closes/retires normally, so the earlier ARM timing/budget failure is not required
for the partial checkpoint. This is diagnostic evidence of a correctness failure,
not a passing product gate or a scheduler fix.

Artifacts outside Git:
- `Projects/sexos-evidence/native-job-cap-finite/result.json`: counts, task/target,
  fixture and input hashes; serial/QEMU logs accompany it.
- `Projects/sexos-evidence/native-job-diag-90e-inputs/provenance.json`: a matching
  sealed normal image and native Web, built incrementally in the same worktree.
- ISO SHA256 `c7470ab3a55c6cc812a020f15c3a7ee9a59030019ad022c052643077d9cc5392`.
- Web SHA256 `05bfd67258e675babbbd8230293fd83663b5cd4ea54036709251edb613438883`.
- ABI-input manifest SHA256
  `942455dc828ffa95c1214639b860311f73efcf9ade0f63876cb0f500d06b5ae8`.

The counter traverses the pinned runtime's pending-job list after the unchanged
10,000-operation limit. It adds no queue scheduling or public API behavior.
This private image is not a release PR. Accepted normal 143 images are separate
and immutable.

Next: run this same instrumented pair against the real HTTPS embed to determine
whether the cap is implicated there. `writeEmbed` causality and YouTube playback
remain unverified. The earlier investigation ledger records the candidate bounded
continuation, owner/cancellation and aggregate-deadline requirements. Do not raise
the cap for one site or run ready/timer events to manufacture progress.

Reference: [HTML microtask checkpoints](https://html.spec.whatwg.org/multipage/webappapis.html#perform-a-microtask-checkpoint)
process the queue to completion; an implementation limit must not silently expose
a partial checkpoint as success.
