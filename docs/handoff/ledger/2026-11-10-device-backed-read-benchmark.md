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
