# Compressed RAM (J5): plan, 2026-11-14

Goal (DAILY_DRIVER_NEXT_100_SLICES J5): a program set whose working set is twice the
RAM still completes, by keeping cold pages compressed in memory instead of failing
with OOM. The name is **Denim** (Andreas's fabric word, proposed): a page packed tight
like denim's weave. Waiting for his OK, like the other proposed names.

## Where SexOS is today

- Program memory is lent eagerly: `SYS_PROGRAM_MEMORY` (81) grows a run's arena page by
  page from `program_frames`, and every lent page is mapped until the run ends. Nothing
  is ever taken back from a live run.
- Under pressure the Shell (J4, PR #190) ends the program holding the most memory after
  5 s at critical. That is the only answer to running out today.
- Demand paging exists only for `sext` demand regions (Bell's probe, Kaleidoscope's
  reserve): a fault in such a region is forwarded to the pager (pd 14), which commits a
  zeroed frame. There is no page-out path.

## Design

The kernel keeps mechanism, Denim keeps policy (DESIGN_GUARDRAILS):

1. **Codec** (`crates/denim`, this PR): `no_std` page compressor. Zero pages and
   same-filled pages cost no storage; other pages use an LZ4-format block. Host-tested
   for round trips, random pages and damaged blocks.
2. **Kernel mechanism, page out:** a new op on syscall 81 lets the memory server take a
   page of a run's arena: the kernel clears the leaf (keeping a "compressed" marker in
   the non-present PTE), flushes the TLB on the cores that run the row, and hands the
   frame's contents to the server through a one-page window. The frame goes back to
   `program_frames`.
3. **Kernel mechanism, page in:** a fault on a marked PTE is forwarded like a demand
   fault (the `forward_page_fault` path), to the Denim server instead of sext. Denim
   unpacks into a fresh frame and commits it with the existing commit op. The faulting
   task stays parked until then, as in demand paging today.
4. **Cold-page choice:** the kernel exposes the accessed bits of a run's arena (read and
   clear, one op on syscall 81, batches of 512 PTEs). Denim scans at low pressure every
   few seconds and pages out what stayed unaccessed for two scans (a clock algorithm),
   starting with the run that has been in the background longest.
5. **Denim server** (`servers/denim`, BOOT service, restartable): keeps compressed pages
   in its own arena (pages of packed blocks, a slab per size class: 256 B to 4 KiB). It
   starts paging out at `PRESSURE_LOW` and stops at normal. If Denim dies, its pages
   are lost, so the runs that had pages there are ended (as a fault would end them) and
   the Shell shows which ones.
6. **J4 stays as the last resort:** the Shell's 5 s grace at critical still ends the
   largest program when compression can't keep up.

## Slices

| # | Slice | Proof |
|---|---|---|
| D1 | `crates/denim` codec, host tests in CI | done in this PR |
| D2 | Kernel: accessed-bit scan op, page-out op with non-present marker, page-in forward | host model + guest gate: a page out/in round trip keeps a program's data |
| D3 | `servers/denim`: store, clock scan, pressure-driven page-out | gate: `heaptest` 2x RAM with a slow access pattern completes |
| D4 | Mesh shows compressed memory per program and the machine's ratio | monitor gate |
| D5 | Soak with Denim on (J2 `--inject` includes Denim) | soak gate |

D2 is the risky part: it touches the page-fault path and TLB shootdown for program
rows, so it gets its own PR with a host model of the PTE states before any guest gate.
