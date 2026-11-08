# Limits and performance: audit, measurements and plan (2026-11-08)

Goal (Andreas): move SexOS past proof of concept to an OS that matches Linux on
capability and performance, then exceeds it. This report lists every hard limit
and performance gap found, measures SexOS against Linux on the same emulated
machine, and sets out staged work. Stage 1 is implemented in the same PR.

Designs stay inside the SASOS rules and the organs plan
(`PROGRAM_SLOTS_DESIGN_2026_11_07.md`): one address space, capabilities decide
authority, PKU keys only enforce, BOOT stays the only source of authority.

## How it was measured

- `apps/bench` (`run bench`, gate `scripts/program_bench_gate.py`) times system
  calls, yields, allocation, copying, file writes, reads and stats with the TSC.
- `tools/bench/linux/` runs the same tests as a static Linux program (`/init` in
  an initramfs) on stock Ubuntu Linux 6.8, with the same QEMU flags the SexOS
  gates use: q35, `-cpu max,+pku`, 512 MiB, an NVMe disk, 1 core.
- Both run under QEMU TCG (this cloud has no KVM). TCG makes some instructions
  far more expensive than on hardware, so ratios matter more than cycle counts,
  and one of them (PKRU writes, below) is distorted. Real-hardware numbers come
  when Andreas's PKU PC is ready.

## Results (cycles per operation, 1 core, after stage 1)

| Test | SexOS | Linux 6.8 | SexOS vs Linux |
|---|---:|---:|---|
| null system call | 32,390 | 1,200 | 27x slower (mostly TCG, see note) |
| yield | 289,688 | 2,962 | 98x slower |
| allocate and touch, per MiB | 4,693,518 | 11,244,562 | 2.4x faster |
| memcpy, per MiB | 432,137 | 2,968,444 | 6.9x faster (see note) |
| file write + sync, per MiB | 1,923,770,370 | 30,309,266 (raw NVMe) | 63x slower |
| file read, per MiB | 662,546,236 | 26,063,132 (raw NVMe) | 25x slower |
| file stat | 409,681 | 23,426 (tmpfs) | 17x slower |

Notes:
- **PKRU under TCG.** A SexOS system call writes PKRU twice (open keys on entry,
  restore on exit). QEMU's TCG takes ~2,700-3,400 cycles per `wrpkru` and flushes
  its software TLB, which the call then refills. Linux never writes PKRU on a
  call. On hardware `wrpkru` is about 20-30 cycles, so this gap is mostly an
  emulator artifact; it must be re-measured on the PKU PC. Stage 1 already
  removed a third, redundant write (null call: 76,703 -> 32,390).
- **Allocation** is faster because a SexOS heap grows in batches of zeroed pages
  granted by one call, where Linux takes one page fault per 4 KiB page.
- **memcpy**: glibc picks AVX2/ERMS paths that TCG emulates slowly; treat this
  as "not slower", not as a real 7x.
- **Files** are the real, large gap: see limits 9 and 10.

## Limits found

Fixed in stage 1 are marked **[fixed]**.

### Memory

1. **Page allocator could hand a page out twice [fixed].** The lock-free buddy
   allocator merged a freed block with its buddy without taking the buddy off
   its free list, so the buddy was in two lists at once. Nothing freed pages at
   runtime yet, so it never fired, but any runtime freeing would have corrupted
   memory. Its free lists also had the ABA problem. Replaced with a buddy
   allocator with doubly linked free lists under one interrupt-safe lock;
   every operation is O(18).
2. **Page tables could be pages the allocator also handed out [fixed].** After
   the buddy allocator took over free memory, the BOOT frame cursor kept giving
   out page-table frames from the same memory, guarded only by marking the 4 KiB
   page, which did not protect a page inside a larger free block. Now every
   frame after BOOT comes from the buddy allocator, and `reserve_frame` carves a
   page out of whatever free block holds it.
3. **TLB shootdowns: one interrupt round per page [fixed].** Ending a program
   sent one synchronous broadcast per page, so a 100 MiB program would have
   needed 25,600 rounds. `tlb::flush_pages` now sends one request that flushes
   everything when more than 8 pages change.
4. **Program memory: 8 MiB per program, reserved at BOOT [fixed].** Each of the
   8 program rows held 8 MiB of RAM from BOOT (64 MiB total, used or not), and a
   program's heap stopped at about 7 MiB. Each row now has an *arena*: address
   space sized at BOOT to the machine's memory (up to 4 GiB), with page tables
   made at BOOT and frames lent from shared memory one page at a time as the
   program grows, returned when it ends. 16 MiB is always kept back for the
   kernel and services. Result on a 512 MiB machine: one program filled 107 MiB
   (was 7), all 8 rows held 12 MiB at once, and every page came back on close.
5. **Program files: 1 MiB [fixed: 16 MiB].** The load region is now 16 MiB, and
   the kernel's private copy is borrowed from free memory per load instead of a
   second permanent buffer. Next: stream the file so no fixed cap remains.
6. **Kernel heap: 256 MiB, all mapped at BOOT.** On a 512 MiB machine that is
   half the RAM; measured use is 15 MiB. Its allocator
   (`linked_list_allocator`) is first-fit and O(n) per allocation.
7. **Quadratic page checks [fixed].** Retire and restore compared every page
   against every other page; fine for 2,000 pages, hours for 1,000,000. Now a
   sort.
8. **Static tables**: per-app page manifests were fixed 2,064-entry arrays in
   statics, copied again for shared-key rows **[fixed: heap vectors]**.
   `service_restart` keeps 2,048 pages per service (8 MiB service images).

### Programs and windows

9. **8 windowed programs**, each a BOOT catalog row; rows from the third on
   share one PKU key and take turns on one core, swapping present bits page by
   page on every switch (cost grows with the program's memory).
10. **800x600 pixel surface** per program row, fixed 4 MiB region, and programs
    cannot learn their window size (Browser thread: `web` draws at 410x330).
11. **Arguments only for programs run from files**, not from the program store.
12. **Program store: 15 programs** (one directory sector), 16-byte names.
13. **No FPU/SSE in programs** (soft-float): tiny-skia thread, PR #18.
14. **One font weight** (Geist Regular; bold and italic are faked): fonts thread.

### Storage

15. **NVMe: one 4 KiB command at a time**, synchronous, one I/O queue
    (`BLOCK_MAX_XFER` 4096). Linux keeps many large commands in flight.
16. **Every file write is a durable commit.** Files commits the copy-on-write
    tree and flushes the disk after each 63 KiB request; Linux buffers and
    flushes on `fsync`. Mounting walks the whole tree.
17. **64 KiB file request region**; the SDK's `files::write` refused files over
    63 KiB **[fixed: it now writes in pieces]**.

### IPC and scheduling

18. **IPC is enqueue + wake, never a direct switch.** A call queues a message
    and unparks the callee; the reply comes back the same way. No L4-style
    handoff, so a round trip costs at least two scheduler passes (yield measured
    at 98x Linux).
19. **The desktop runs on core 0.** Shell and Display stay there; shared-key
    program rows all run on one core.
20. **Scheduler**: FIFO work-stealing queues of 512, 1 kHz tick, no priorities
    or deadlines.

### Other fixed sizes

`text_batch` 6 KiB, documents 12 KiB, network job 4 KiB per message
(32 KiB duplex), Spindle view 110x48, 128 cores, 1,024 capability slots per
domain. `kernel/src/benchmark.rs` prints fixed claims ("3.4x faster than
Linux") rather than measurements; it should be deleted or made real.

## Plan

Each stage is a PR, merged when its gates pass at 1 and 4 cores.

1. **Memory correctness and program memory** (this PR): allocator, page tables,
   batched shootdowns, arenas, 16 MiB files, bench tools. ABI changes, named:
   `program_store::IMAGE_BYTES` 1 -> 16 MiB, `WINDOW_BYTES` 8 MiB -> 4 GiB
   (maximum arena), `program_memory` grants now come from the arena (the base
   address moved; contiguity and semantics are unchanged), `heaptest` and
   gates updated.
2. **Kernel heap sized to the machine.** Start at 32 MiB and grow on demand by
   mapping buddy pages; replace `linked_list_allocator` with a size-class slab
   over the buddy allocator (Linux's SLUB model).
3. **Storage throughput.** NVMe PRP lists for up to 128 KiB per command, a queue
   depth of 32 with completions by interrupt, then a second queue per core.
   Files: commit on `SYNC` and on a short timer instead of per request, and a
   shared buffer per client so data is not copied through the 64 KiB region.
   Target: within 2x of Linux raw NVMe on the bench.
4. **Windows that fit the screen.** A window-size event and query for programs,
   pixel surfaces sized to the framebuffer (shared, revocable, per window),
   arguments for store programs, a multi-sector program store.
5. **Programs without fixed rows.** Use the existing generation scheme to make
   rows reusable without limit, organ keys per `PROGRAM_SLOTS_DESIGN`, and
   tenancy switches at page-directory level (one entry per arena, not one per
   page). Stream program loading.
6. **IPC fast path.** Direct handoff for call and reply on the same core (the
   caller donates its time slice to the callee and back), shared-memory rings
   for bulk data, then Shell and Display off core 0.
7. **Scheduler and SMP**: priorities and deadlines for input and display,
   per-core page caches, PCID so a switch does not flush the TLB.
8. **Real hardware**: re-run the bench on the PKU PC; XSAVE/AVX after PR #18.

## Progress

- Stage 1 merged (PR #21). One program went from 7 MiB to 107 MiB on a 512 MiB VM.
- Stage 2, first half (PR #23): the kernel heap starts at 32 MiB and grows on demand.
  One program fills 323 MiB of 512 MiB. The slab is still to come.
- Large machines (follow-up PR): every gate used to run at 512 MiB, but real machines
  have 4-32 GB. At 8 GB one program filled its full 4 GiB arena. Changes:
  - Arena page tables are made on first use, not at BOOT. The tables for 4 GiB had cost
    8 MiB per row.
  - Arenas are sized to the machine's memory. ABI, named: `WINDOW_BYTES` 4 GiB -> 256 GiB
    (the maximum arena).
  - The VA allocator ends at the kernel heap's fixed range (0x4444_4444_0000). Before,
    a large enough reservation could have run into it.
  - The kernel heap's growth limit is a quarter of RAM, at least 1 GiB (VA up to 64 GiB).
  - At 8 GB, program records use about 90 MiB of kernel heap (4,096 records, each with a
    16 KiB kernel stack). That's by design, about 1% of RAM.
- Program stacks (same PR): the stack moved from BOOT's fixed 64 KiB to the top of
  the arena. Its size comes from the file (`-z stack-size`, PT_GNU_STACK); without one
  it's 1 MiB, and the most is 256 MiB. An unmapped guard page sits below it.
  heaptest proves that an overflow ends only that run.

- Stage 3, first part (storage):
  - The NVMe self-test busy-polled a fixed 1,000,000 times, so a slow or busy host
    sometimes saw `readback_timeout` and storage stayed off for that boot. It now uses the
    time-bounded `completion_wait`, like every other I/O path.
  - Each command used to sleep a whole tick after 64 polls. It now yields first (256
    turns) and sleeps only after that, and the per-wait log line is written only for slow
    waits.
  - Bench, cycles per MiB: file write 1.89G -> 0.63G (3x), read 660M -> 205M (3.2x).
  - Files write-back: a WRITE no longer commits. Commits happen on SYNC, before any other
    change, at 8 MiB uncommitted, and after 20 idle ticks. A read error no longer rolls
    back other writes. This removed 130 of 134 flushes for a 4 MiB write, but the bench
    time didn't change.
  - What remains is about 2.5M cycles per 4 KiB block command (IPC, DMA bounce and key
    switches under TCG). Next: larger transfers per command (`BLOCK_MAX_XFER` past
    4 KiB, PRP lists), sexfs extents, and queue depth.
  - Also found: `fs_service` keeps its free-space bitmap in memory, sized for 16 GiB
    (`MAP_WORDS`), and caches only 64 blocks. Both are fixed limits to remove.

- Stage 3, second part (large disk commands):
  - One NVMe command now moves up to 128 KiB (`BLOCK_MAX_RUN`). sexdrive's two bounce
    leases are each 128 KiB plus a page holding the command's PRP list. Two pages use
    PRP2 directly. `BLOCK_MAX_XFER` (4 KiB) is unchanged for its existing page-sized users.
  - ABI changes: new `sex_pdx::BLOCK_MAX_RUN = 128 KiB`; BOOT's Files/drive block
    buffer (SLOT_BLOCK_BUFFER, region 4) grows from 4 KiB to 128 KiB; the block
    protocol accepts READ/WRITE up to 128 KiB.
  - sexfs: `Device` gains `run_blocks`, `read_run` and `write_run` (defaults loop over
    blocks). Whole-block file writes take side-by-side free blocks and write them with
    one command; reads of side-by-side blocks use one command and check every block's
    checksum. A partial write after progress now ends short inside sexfs itself.
  - Files copies each READ and WRITE through its own buffer, so checksums cover bytes a
    client can no longer change.
  - Bench, cycles per MiB (1 core): file write 0.63G -> 107M (5.9x), read 205M -> 49M
    (4.2x). Against Linux on raw NVMe: write 3.5x slower (was 63x), read 1.9x slower
    (was 25x). A request is still at most 63 KiB (the client region), so a 1 MiB write
    is 16 requests; a larger client region and queue depth are the next steps.

- Stage 3, third part (service memory, filesystem size):
  - BOOT services had only a fixed 128 KiB heap, so Files kept a 16 GiB free-space map
    and a 64-block cache in its ELF. A BOOT service may now get an arena like a program
    row's (`kernel/src/service_memory.rs`; BOOT gives one to Files). SYS_PROGRAM_MEMORY
    grows it page by page, from the same frames programs borrow.
  - sex-rt's allocator falls back to those pages once its fixed arena is full. A domain
    without an arena keeps the old bound, which costs one denied call, once.
  - Files sizes its free-space maps to the disk it finds (a bit per 4 KiB block in each
    of two maps: 64 MiB for a 1 TiB disk) and caches 1024 tree blocks. When memory can't
    grow, it falls back to the old fixed sizes.
  - ABI change: SYS_PROGRAM_MEMORY (81) also serves BOOT services that have an arena.
  - A commit now walks only the words of the pending map it touched, not the whole map.
  - Tested: a 1 TiB disk (4 GB VM) formats, mounts, survives a power cut and reports
    1,048,512 MiB; Files grew to 68 MiB.
  - A disk command may now take 30 s before it counts as lost (was 1 s), as on Linux.
    Under host load a flush took up to 1.4 s, which used to quarantine the drive.
  - Long term, free space should live in the tree (extents), so memory stays flat
    however big the disk is.

Where SexOS can exceed Linux: allocation is already faster (batched grants, no
fault per page); one address space makes zero-copy sharing a capability grant
instead of a copy, so with stages 3 and 6 file and IPC data should never be
copied; and PKU switches are tens of cycles on hardware against a Linux
process switch's page-table change.
