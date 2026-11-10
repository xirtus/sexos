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

- Stage 3, fourth part (request size):
  - A request could end in the middle of a block, because `DATA_MAX` (the region less
    its 1 KiB path area, so 63 KiB) is not a whole number of 4 KiB blocks. That cost
    Files a read-modify-write of one block per request. New `fs::IO_MAX` is `DATA_MAX` rounded down to whole blocks, and sex-app
    reads and writes in IO_MAX pieces.
  - The file region a program shares with Files grows from 64 KiB to 256 KiB, so one
    request carries up to 252 KiB.
  - ABI changes: `fs::REGION_BYTES` 64 KiB -> 256 KiB (and with it `DATA_MAX`); new
    `fs::IO_MAX`.
  - Bench, cycles per MiB (1 core): file write 107M -> 67M, read 49M -> 26M. Against
    Linux on raw NVMe: read is now even (26M), and write is 2.2x slower (30M).
  - Next for writes: queue depth (several commands in flight), so a request's runs
    don't wait on each other.

- Stage 3, fifth part (fast memory copies):
  - memcpy, memmove and memset were byte-by-byte loops in the kernel, sex-rt (Files and
    every app), sexdrive and spindle. Each disk request is copied twice (program region
    to Files, Files to the DMA bounce buffer), so the loops dominated write cost.
  - New `sex_pdx::mem`: `rep movsq` + `rep movsb` copies, a backward `rep movsb` for
    overlapping moves, `rep stosq` fills. On CPUs with ERMS/FSRM these are the fastest
    general copy, as in Linux's `memcpy`. A host test checks them against std at many
    sizes, offsets and overlaps (`tests/sex_pdx_mem.rs`).
  - No ABI change.
  - Bench, cycles per MiB: 1 core write 67M -> 31M, read 26M -> 15M; 4 cores write
    41M -> 34M, read 29M -> 23M. Against Linux on raw NVMe (30M write, 26M read):
    write is now even, and read is 1.7x faster.
  - Next: queue depth (several commands in flight).

- Stage 3 is done: on the bench, file writes match Linux and reads beat it, past the
  "within 2x" target. Queue depth (several disk commands in flight) moves to stage 8,
  where it can be measured on a real NVMe drive: under QEMU's software CPU, the
  emulator's own cost per command hides what overlap would save.

- Stage 4, first part (windows that fit the screen):
  - Program pixel surfaces were a fixed 4 MiB (800x600). BOOT now sizes each to the
    screen (4 bytes per screen pixel, rounded up to 1 MiB, never under 4 MiB), so a
    full-screen window can be all pixels. Display accepts any shape the region holds
    and clips pixels to the window's border instead of capping at 800x600.
  - Programs learn their window size. Whenever Shell gives a program window a new
    size (tiling, maximize, drag resize), it queues a resize input event with the
    pixel area, in order with the program's keys. sex-app adds `Event::Resize`
    and `Window::size()`, and `Window::pixels()` covers the whole region.
  - View fits a picture to its window and fits it again on every resize (the gate
    checks that the last picture fits the last size Shell reported).
  - ABI changes: `pixel_region` region length is now at least `BYTES` (read it from
    the grant); `pixel_region_display` and `pixel_region_client` return
    `(address, length)`; new input class `pixel_region::RESIZE_CLASS` (6) on opcode
    0x202 with `resize_wire`/`resize_decode`; `pixel_region::area`, `LEFT`, `TOP`;
    `sex_app::Event::Resize`.
  - Web still draws at the 410x330 its gates compare against the host render; it can
    now follow `Event::Resize`, which needs the gates to render at the reported size.

- Stage 4, second part (the program store):
  - Programs in the disk program store took no arguments: `run NAME ARGS` looked only
    for `/bin/NAME`. Now Files prepares a `/bin/NAME` run even when no such file
    exists, and the load falls back to the store's NAME with the same arguments.
    `run jot @todo.txt` runs a store Jot on that file.
  - The store's directory was one sector (15 programs). Its head now says how many
    sectors it spans, up to 64 (960 programs); old one-sector stores read as before.
    `scripts/install_program.py` writes as many sectors as it needs.
  - ABI changes: program store directory head bytes 8..12 are the sector count;
    `program_store::MAX_DIRECTORY_SECTORS`, `MAX_PROGRAMS`, `directory_sectors`;
    `fs::PREPARE` accepts a missing `/bin/NAME`.
  - Gate: `scripts/program_store_gate.py` (19 programs, Jot in the second sector, run
    bare and with an `@file` argument, a missing name still refused).
  - Files in `/bin` already had no count limit; the store is for programs installed
    with the system.

- Stage 5, first part (switching between programs on the shared key):
  - Program rows from the fourth on share one PKU key and take turns on one core.
    A switch used to clear and set the present bit of every page both programs held
    (1,100+ region pages each, plus their memory), so it grew with program size.
    Worse, the region pages went into a fixed 1,400-entry table: once stage 4 sized
    pixel surfaces to the screen, a screen above about 1400x1000 overflowed it and
    BOOT panicked as soon as these rows existed. Andreas's PC would have hit this.
  - Now every page such a row holds lies on whole 2 MiB spans nobody else maps into:
    its arena already did, and BOOT now places its regions on span boundaries. A
    switch flips the user bit of the spans' page-directory entries (6 spans for a
    row's regions at 1920x1080, plus one per 2 MiB of program memory) and flushes
    this core's TLB. BOOT's 64 KiB first stack for the row is outside any span and
    still takes turns page by page.
  - Kernel checks of user pointers now honour the user and write bits at every
    paging level, as the CPU does, so a closed span is closed to system calls too.
  - Measured: ~190,000-310,000 cycles per switch under TCG, almost all of it the
    full TLB flush (QEMU drops its whole software TLB); constant in program size.
    On hardware this is a few page-directory writes plus a TLB refill; PCID
    (stage 7) can remove the flush.
  - No ABI change. Gate: `scripts/program_pool_gate.py` (six programs side by
    side at 1920x1080, four on the shared key, all drawn, switches counted).

- Stage 6, first part (IPC: call and wait, reply and wait, direct handoff):
  - A round trip to a service took seven kernel entries: call, an empty listen,
    a wait, the service's listen and reply, its empty listen and wait, then the
    caller's listen; and each woken side waited its turn behind the whole queue.
  - Two new system calls do a step and its wait in one entry: `PDX_CALL_WAIT` (100,
    syscall 0 then 53) and `PDX_REPLY_WAIT` (101, syscall 29 then 53). The wake either
    causes is a handoff: if the partner lives on this core it runs next, ahead of the
    queue, so a round trip is two switches with no queue wait. Partners may take
    turns ahead of the queue 8 times in a row, then everything queued runs first, so
    a busy pair cannot starve the desktop.
  - Used by `sex_app::files` (every file request) and by Files when it has nothing
    else owed or uncommitted. New `fs::PING` request, answered at once.
  - Bench (`ipc_roundtrip`, 1 core, TCG): SexOS 392,117 -> 255,906 cycles per round
    trip (1.5x); `file_stat` 438,089 -> 335,941. Linux 6.8 pipe round trip between
    two processes (`tools/bench/linux`, same machine): 226,366. So SexOS IPC is now
    level with a Linux pipe under emulation, where each kernel entry costs ~30,000
    cycles because QEMU emulates `wrpkru` slowly. On hardware a SexOS round trip is
    four kernel entries and two switches without a page-table change, against two
    page-table switches for Linux; the 10x target is measured there.
  - Found and fixed at 4 cores: a task parked waiting for a message stays attached
    to its core, and a wake from another core only marked it pending. If that core
    was halted, it looked again only at its next timer interrupt, so every call to a
    service on another core cost about one tick. Such a wake now sends the owning
    core a wake interrupt. 4 cores: `ipc_roundtrip` 2,731,734 -> 396,167 cycles
    (6.9x), `file_stat` 2,620,806 -> 494,010, `file_read` 24.6M -> 14.8M per MiB.
  - Next: deliver the message in the waiting call's registers (no listen after the
    wake, two entries per round trip).
  - ABI changes: system calls 100 (`sex_pdx::pdx_call_wait`) and 101
    (`sex_pdx::pdx_reply_wait`), `sex_pdx::fs::PING` (21), `sex_app::files::ping`.

Where SexOS can exceed Linux: allocation is already faster (batched grants, no
fault per page); one address space makes zero-copy sharing a capability grant
instead of a copy, so with stages 3 and 6 file and IPC data should never be
copied; and PKU switches are tens of cycles on hardware against a Linux
process switch's page-table change.

### Memory pressure signal (2026-11-09)

The kernel now tells programs and services how short the machine is on memory before an
allocation fails. `program_frames::pressure()` compares free pages (above the 16 MiB
floor) with the most that were ever free: LOW below an eighth (at least 32 MiB),
CRITICAL below a thirty-second (at least 8 MiB). A level rises at once and falls only
when free memory is a quarter above the mark, so nothing flaps. Transitions log
`[memory.pressure] level= free_pages= base_pages=`. It is advisory: nothing is killed.

- ABI addition: syscall 81 (`SYS_PROGRAM_MEMORY`) now returns the level in r10 for
  every caller, whatever the status (r10 was clobbered before, so old callers are
  unaffected). `sex_pdx::program_memory::{pressure, PRESSURE_NORMAL, PRESSURE_LOW,
  PRESSURE_CRITICAL}`.
- Gate: program_heap. Heaptest reports the highest level seen while filling memory; at
  512 MiB it reaches critical (level 1 at 10283 free pages, 2 at 2570 of 82272), and after
  Close the kernel logs level 0 and the next run starts at normal. Green at 1 and 4 cores.
- Next: consumers. The browser drops decoded images and caches on LOW, Play shrinks
  buffers, and Desk shows a warning on CRITICAL.

### Stage 7a: latency classes and wake preemption (2026-11-10)

Each core now has two run queues. Interactive tasks (sexdisplay, silk-shell, sexinput,
sexusb, silkbar) that are woken go to an urgent queue, taken ahead of the normal one,
at most 16 times in a row while a normal task waits. A task switched out still runnable
(its time ran out, or it yielded) always waits in the normal queue, so a desktop
service that polls cannot starve the rest of its core. When an interactive wake lands
on a core running a normal task in user code, the waker sends that core (itself
included) the new `PREEMPT_VECTOR` (0xF2); its handler shares the timer's switching
stub, so the normal task gives way at once instead of at its tick. Sound (sexbell)
stays normal: its device ring covers a scheduling delay, and it needs a deadline class.

The kernel counts wake-to-run latency per class (TSC cycles, a power-of-two histogram)
and logs `[sched.latency] class= wakes= avg_cycles= max_cycles= preemptions= hist=`
every 2048 ticks. New gate `latency_classes`: `run bench spin` computes for ~20 G
cycles with no system call while the pointer moves, then the normal bench runs.

| 1 core, spinner on the desktop's core (TCG) | before | after |
|---|---|---|
| interactive wake, average | 359k cycles | 130k cycles |
| interactive wake, p50 | <131k | <66k |
| interactive wake, p90 | <1.05M | <262k |
| interactive wake, p99 | <4.2M | <2.1M |

At 4 cores programs run on cores 2 and up, away from the desktop's core 0, so the
classes matter less there; the gate still checks the counters. Normal tasks wait a bit
longer on a shared core, which is the trade Linux's wakeup preemption makes too.

- ABI: none for programs. Kernel-internal: vector 0xF2; `interrupts.rs` is an ABI
  input, so `abi_version_hash` is recomputed.
- Gates green at 1 and 4 cores: latency_classes, program_bench, program_file,
  program_disk, program_store, filesystem, program_pool, program_heap, program_launch,
  program_threads, program_view.
- Next (7b): a deadline class for sound and video frames; latency classes for programs
  that own the focused window; PCID so a switch keeps the TLB.

### Head-to-head with Linux (2026-11-10)

Same emulated machine for both (QEMU TCG, q35, `-cpu max`, NVMe, 512 MiB). Linux is
the Ubuntu 6.8 kernel with `tools/bench/linux/bench.c` as `/init`
(`tools/bench/linux/run.sh vmlinuz CORES mods/`); SexOS is `run bench`
(`scripts/program_bench_gate.py`). Cycles per operation, lower is better.

Under TCG every `wrpkru` costs ~3,000 cycles (Linux measures 2.7k–3.7k) because the
emulator flushes its TLB; on a real PKU CPU it is ~20–30 cycles. A SexOS system call does
two or three, so "SexOS, PKU emulated" is dominated by the emulator. "SexOS, no PKU"
boots with `SEXOS_GATE_CPU=max,-pku,+rdseed`: the same kernel and services without
the key writes, which is close to what real hardware pays (add ~75 cycles a call).

| 1 core | Linux 6.8 | SexOS, no PKU | SexOS, PKU emulated |
|---|---|---|---|
| null system call | 1,175 | **721** | 29,348 |
| IPC round trip (Linux: pipe) | 218,249 | **33,511** | 350,547 |
| Linux futex round trip (threads) | 73,975 | | |
| Linux Unix socket round trip | 451,970 | | |
| allocate + touch, per MiB | 9,890,972 | **2,641,614** | 2,735,243 |
| memcpy, per MiB | 2,772,307 | **242,118** | 290,736 |
| write 4 MiB + sync, per MiB | 27,173,658 | **15,511,307** | 26,995,628 |
| read 4 MiB, per MiB | 24,303,475 | **6,998,589** | 14,503,695 |
| one 4 KiB read | 626,300 | **385,774** | 905,564 |
| file stat | **39,866** | 53,128 | 302,878 |
| power-on to first program (TSC) | **7.46 G** | 10.18 G | 10.62 G |

| 4 cores | Linux 6.8 | SexOS, no PKU | SexOS, PKU emulated |
|---|---|---|---|
| null system call | 1,251 | **866** | 43,994 |
| IPC round trip (Linux: pipe) | 757,905 | **166,617** | 339,748 |
| Linux futex / Unix socket | 408,103 / 821,835 | | |
| read 4 MiB, per MiB | 49,902,067 | **22,503,351** | 27,685,289 |
| one 4 KiB read | 627,768 | 1,005,053 | 1,499,044 |
| file stat | **23,018** | 240,723 | 285,853 |

Caveats, plainly:
- Linux's disk tests use the raw block device (O_DIRECT for 4 KiB); SexOS's go through
  its filesystem and may hit Files' cache for the 4 KiB reads.
- Linux's memcpy is glibc's AVX path, slow under TCG; SexOS uses `rep movsb`.
- Linux boots with `-kernel` (no bootloader); SexOS boots Limine from the ISO, so its
  power-on number includes a bootloader Linux skips.
- `yield` is not compared: Linux's initramfs has nothing else to run, SexOS's desktop does.

Where SexOS is behind, and the plan:
1. **file stat (1.3x slower on 1 core, 10x on 4) [fixed: stat cache, below]:** a stat is a call to Files on
   another core; Linux answers from its own dentry cache in the caller's system call.
   Plan: Files publishes a read-only metadata map (sizes, times, generation) that
   programs read without a call, the way Linux's vDSO serves the clock; a call only on a
   miss. Also stage 6b (reply in registers on wake) and services per core.
2. **one 4 KiB read at 4 cores (1.6x slower):** same cross-core trip to Files and
   sexdrive. Plan: per-core NVMe submission queues (stage 8) and the same
   services-per-core work.
3. **boot (36% more cycles to the first program):** part is Limine + ISO. Plan: measure
   from kernel entry, start services in parallel on all cores, and defer probes that
   aren't needed for the first frame.
4. **PKU switch cost on real hardware:** three `wrpkru` per call; stage 6b drops the
   exit write when a call returns to the same domain, and the PKU PC will give real
   numbers.

### Stat cache: a stat without a call (2026-11-10)

Each client's 256 KiB Files region now ends in a 4 KiB `stat_cache` (64 entries of
64 bytes; `sex_pdx::fs::stat_cache`). Files fills an entry when it answers a STAT,
keyed by the path as the client spelled it, and clears entries in every client's
region before a change can be seen: by file for WRITE / TRUNCATE / CREATE / MKDIR, all of
them for REMOVE, RENAME, MOUNT, UNMOUNT, a rollback or a failed commit, and a client's
own entries when it binds. Each entry is seqlocked (odd while Files writes it), so a
reader never takes a half-written answer. `sex_app` (`size`, `stat`, `direct::stat`)
reads the cache first and calls Files only on a miss. The bench checks that a size read
after a write is the new one.

| file stat, cycles | Linux 6.8 | SexOS before | SexOS now |
|---|---|---|---|
| 1 core, no PKU | 39,866 | 53,128 | **2,700** |
| 4 cores, no PKU | 23,018 | 240,723 | **4,049** |
| 1 core, PKU emulated | | 302,878 | 4,169 |

Stat is now 15x faster than Linux on 1 core and 5.7x on 4 cores (TCG).

**ABI:** `fs::DATA_MAX` and `fs::IO_MAX` shrink by 4 KiB (CACHE_BYTES); the region's
last 4 KiB (`CACHE_AT`) belong to Files. Programs built against the old sizes would
send requests that overlap the cache; all in-tree clients use the constants.


### Boot time, split at kernel entry (2026-11-10)

The kernel logs `[boot.kernel_entry] tsc=` first thing, so the boot number splits into
firmware + loader and kernel. On 1 core without PKU (TCG):

| | before | now |
|---|---|---|
| power-on to kernel entry | ~7.1 G (3 s menu countdown) | 5.76 G (1 s countdown) |
| kernel entry to first program | ~3.0 G | 3.04 G |
| power-on to first program | 10.18 G | **8.80 G** (Linux 7.46 G, without a loader) |

- The Limine menu now waits 1 s instead of 3 (`limine.cfg`); its entries can still be
  picked (a gate choosing the 1024x768 entry was checked).
- Excluding the loader, SexOS reaches its first program in 3.0 G cycles; Linux's 7.46 G
  includes firmware and decompressing the kernel, so the kernel part is not behind.
- The kernel writes 263 KB of serial log before the first program. On a real 115200-baud
  UART that alone would take over 20 s. Next: keep boot proof lines but send the bulk to
  the crash/boot log buffer, not the UART, on real hardware.
