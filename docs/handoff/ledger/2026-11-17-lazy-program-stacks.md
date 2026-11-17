# Lazy program stacks (issue #306)

A program run now starts with only the top 64 KiB of its stack (`STACK_COMMIT`, 16
pages) instead of all of it (1 MiB by default, up to 256 MiB with `-z stack-size`).
The rest of the reserved stack is committed a zeroed page at a time when the run first
touches it: the fault comes to the kernel's own stack check (`app_resources::stack_fault`)
before any pager, and the task goes on at once. The unmapped guard page under the
reservation stays: a touch there logs `[app.program.stack_overflow]` and the run ends as
any bad fault ends it, with a crash record.

Lazily committed pages live in their own list (`App::stack`), not the run's manifest, so
heap pages stay the manifest's last `grown` entries and the heap index math (Denim, VM
loans, SEAL/OPEN) is unchanged. Close, a forced end and retire take them with the rest.

Programs built for `x86_64-sex-app.json` now get inline stack probes, as ordinary programs
(`x86_64-sex-std.json`) already did, so a frame bigger than a page touches every page in
order and cannot jump over the guard page into the heap.

Gate: `scripts/program_heap_gate.py` (heaptest asks for 4 MiB: `stack_pages=1024
stack_committed=16`; S runs 1.25 MiB deep; O overflows into the guard page and ends with a
crash record). Mesh shows a run's committed pages, stack included.
