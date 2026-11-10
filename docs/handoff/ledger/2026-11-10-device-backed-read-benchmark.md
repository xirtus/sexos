# Measure device-backed large reads before changing NVMe queues

The earlier limits handoff says its small-read benchmark may be served by Files'
cache. Source inspection gives a more precise boundary: `sexfs` caches tree blocks;
`read_block` and `read_run` submit file payload to `Device`. Hardware/controller and
host caches remain outside this boundary. No queue optimization is justified by
assuming either path without measuring it.

`run bench read-large` creates and syncs a 16 MiB patterned file in the benchmark's
own folder. Three sequential 64 KiB-read passes verify every byte and report read
cycles, whole-pass elapsed ticks, and successful payload device commands/bytes.
Per-read timing excludes the subsequent data verification loop. Each pass requires
256 READ requests and exactly 16 MiB of payload device I/O. Sparse zeros, metadata
traffic, another client's I/O, and a server counter reset cannot qualify a pass.
No arbitrary disk access or extra filesystem grant is given to the benchmark.

Files accounts completed payload reads through explicit default-compatible Device
hooks, separately from metadata. The existing authenticated Files route and private
client Region expose only that client's READ counters; a program binding clears the
row for its new run. The new protocol selector is a server/SDK change, with ABI pins
regenerated, and no kernel changes. Counters include payload blocks read during a
READ which later fails validation; the benchmark also requires every request and
byte comparison to succeed. Non-filesystem/raw-volume I/O is not claimed covered.

The guest benchmark is added to the existing required runtime CI job, with the same
evidence artifact. Local host validation: all eleven actual filesystem model,
crash, damage and data-read tests pass; two strict wire/reset cases pass; bench and
Files cross-target checks pass. Guest qualification and exact-head CI are pending.
This is driver-submission evidence, not physical-media cache misses, a Linux
comparison, or evidence that per-core NVMe queues improve throughput.

## Qualified local guest runs

A clean sealed normal entrypoint build of `bf660ce4` passes the new gate on
one and four QEMU TCG cores, PKU enabled, 1280x800. Every one of the six passes
verifies all 16,777,216 bytes and records 256 READ requests, 256 successful payload
device commands and exactly 16,777,216 device bytes. Every run begins with zero
private counters after file creation/sync. Both guests were stopped and reaped.

One-core read cycles: 189,677,000 / 187,301,000 / 210,269,000; four-core read cycles:
146,882,000 / 122,328,000 / 121,567,000. These are raw sequential-program baseline
observations, not an isolated queue comparison or a hardware throughput claim.

Immutable ISO SHA-256:
`85e8d1a10f848c8622b403ccffaa7df0983e41afb02e0699e053a704bb2d65ff`;
benchmark SHA-256 `044e563a1fa9f69d5f2c80ee81b92d2932df131bd5fa46f4b755365b1eeb0f9a`;
Files SHA-256 `851e1da457050689e15615242bea4dceb67b698f2eb6ddf8c4771104ed08ecca`.
ABI, runner and bootloader hashes, raw serial, JSON results, disk inputs and build
log are in `Projects/sexos-evidence/pr142-device-read-bench/`. The runner uses the
already-qualified #135 boot cleanup helper, pinned separately in provenance;
benchmark assertions are unchanged. This isolated normal build has no FFmpeg
archives installed and is not presented as a media release image. Exact-head CI
remains required before merge. The full recording/UX candidate will be separate.
