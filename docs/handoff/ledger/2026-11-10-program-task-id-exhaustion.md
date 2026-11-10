# Program run/worker task identities fail closed

The shared run/worker allocator previously used fetch_add and rejected the returned u32::MAX after the counter had already wrapped to zero. A later call could reuse a BOOT identity. It also had no boundary against the high-half task namespace.

Run/worker IDs now remain in [0x40000000, 0x80000000): the last valid ID is 0x7fffffff, and every later allocation fails without changing the counter. Unexpected counters below that range or above its end also fail. BOOT main tasks use their domain IDs, BOOT trampolines use domain|0x80000000, and the separate program-record allocator starts at 0x80000000. This change confines the run/worker allocator; it does not change or assert a defect in the separate record allocator.

Both workers and reusable-row rebirth use the same checked atomic reservation. Rebirth computes its next generation, reserves an ID, then changes generation; exhaustion cannot half-rearm the original task or mutate its app generation/manifest/arena/receipts. Existing loader failures after approved-source matching retain their documented behavior; this is not a general transactional loader change.

Net kernel growth: four nonblank lines, no recorder policy or new capability. ABI inputs are regenerated with scripts/update_abi_hashes.sh.

Production-module tests pass for concurrent final-range allocation, repeated boundary/MAX/lower-range exhaustion, unchanged actual rebirth snapshots, and unchanged worker runs. Compiled wrapping, namespace-bypass and early-generation mutation controls all fail at runtime. Full app resource host gate (including existing source-pin controls), the sex-kernel custom-target check and design guardrails pass. Exact-head native reload evidence remains required before readiness.
