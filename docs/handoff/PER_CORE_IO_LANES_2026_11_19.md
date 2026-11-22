# Per-core zero-copy disk and network paths (I/O lanes), 2026-11-19

Thread "Add per-core zero-copy disk and network paths". Builds on "Speed up native
drivers" (NIC interrupts #451, NVMe queue depth 32 + 4 MiB runs, filesystem layout),
which keeps those pieces; this work is what sits on top.

## Where the time goes today (measured, CI KVM lane, AMD EPYC 9V74, median of 3)

| bench (`program_bench_gate --accel kvm --no-pku`) | 1 core | 4 cores | 4 / 1 |
|---|---|---|---|
| `file_read` 4 MiB, cycles per MiB | 5,057,988 | 12,137,424 | 2.4x slower |
| `file_read_4k`, cycles per read | 283,596 | 665,458 | 2.3x slower |
| `ipc_roundtrip`, cycles | 11,944 | 94,401 | 7.9x |

More cores make disk reads *slower*. With 4 cores, Files (pd 11) and SexDrive (pd 2)
run on core 3 and the bench on another core, but every NVMe interrupt is aimed at
core 0 (`device_irq::record` used the boot core's APIC for every vector): core 0 takes
it, sets a bit and IPIs core 3 to wake SexDrive. Each request also crosses cores twice
(app to Files, Files back), and every byte is copied twice (Files' block buffer into
the shared window, the window into the app's buffer).

## Design

A **lane** is one core's private path: a submission ring and a completion ring in
memory both sides see (one address space, so a request names its buffer by address
and nothing is copied), a device queue of its own (NVMe I/O queue pair, NIC RX/TX
pair), and that queue's MSI-X vector aimed at the same core, waking a driver worker
that runs there. No lock is shared between cores.

1. **Kernel mechanism (this PR, about 130 kernel lines)**
   - `OP_UNMASK` aims the vector at the unmasking task's core (direct MSI/MSI-X entries
     only; VMD-routed entries stay put). SexDrive and SexNet take their own interrupts
     on their own core: no IPI hop.
   - `SYS_DEVICE OP_STEER` (86/7; 6 is device power): aim one vector at any core.
   - `SYS_DEVICE OP_BIND` (86/8): a vector signals a bit of an IPC v2 notification the
     driver holds instead of waking the main task; each queue's worker waits on its own
     notification (`SYS_NOTIFY OP_NOTIFY_WAIT`). A destroyed notification falls back.
   - `SYS_PROGRAM_THREAD SPAWN_ON` (83/7): a driver service (a domain holding a device
     lease) starts a worker on a chosen core. Workers share the service's domain, key
     and authority; they end with the service (`program_workers::end`).
2. **`crates/sex-lanes` (host-tested, `scripts/lanes_host_gate.sh`)**
   - `ring`: SPSC rings over shared memory with the need-wakeup handshake (a busy
     consumer costs the producer no system call), robust to a lying peer (cursors that
     no honest peer can produce read as `Corrupt`, never out of bounds). A two-thread
     test moves 2 million items with the consumer sleeping whenever empty.
   - `poll`: adaptive polling. A burst masks the vector and polls; 64 empty rounds
     unmask and sleep; a stretch yields every 4096 rounds; the burst size that enters
     polling adapts between 2 and 32.
   - `nvme`: Set Features Number of Queues, Create/Delete I/O CQ/SQ, doorbell stride.
   - `plan`: core to queue to vector when a device grants fewer queues or vectors than
     cores (NVMe keeps vector 0 for admin, as Linux does).
   - `rss`: Toeplitz hash (passes Microsoft's verification vectors), indirection table,
     Intel igb/igc multi-queue registers.
3. **NVMe one queue pair per core** (after the NVMe depth PR merges): SexDrive asks
   for one queue per core, creates CQ/SQ pairs each with its own vector steered to its
   core, and runs a worker per queue that drains its lane and its CQ.
4. **NIC queues per core** (after #451): RSS on igb/igc, one RX/TX pair per core,
   SexNet workers per queue; virtio-net multiqueue for QEMU.
5. **Zero copy**: a client lends its buffer (region view) once; Files hands the block
   run's address to SexDrive, which DMA-lends it through the IOMMU (syscall 90), so
   the device writes straight into the app's buffer. Packet pools shared read-only
   with the socket owner for receive.

## Isolation

Rings live in memory lent between exactly two domains; each side validates
everything it reads (cursor ranges, buffer addresses inside the lent region). DMA
into a client buffer goes through the driver's IOMMU domain only for the lent range.
Workers run under the service's own PKU key. No new authority is minted: a worker can
do only what its service could.

## ABI

- Syscall 86 `SYS_DEVICE`: new ops 6 `OP_STEER`, 7 `OP_BIND` (4 and 5 are the touchpad
  thread's I/O APIC lines).
- Syscall 83 `SYS_PROGRAM_THREAD`: new op 7 `SPAWN_ON` (driver services only).
- `OP_UNMASK` now also re-aims the vector at the caller's core.

## Finding: per-core queues need more than one submitter

SexDrive serves one request at a time on one task, and its only client is Files,
which is also one task making one synchronous call per block run. Every disk read in
the system therefore funnels through one Files task and one SexDrive task, whatever
core the app runs on. One NVMe queue per core only pays once several cores submit at
once, so the queues alone would measure as no change. What does pay today:

- interrupts on the driver's own core (stage 1, #457);
- spin-then-sleep for synchronous commands (`poll::Hybrid`, SexDrive, stacked on the
  queue-depth branch);
- QD32 batches and 4 MiB runs (the "Speed up native drivers" thread).

## Stage 3 proposal: direct read lanes (Files out of the read data path)

The design that makes per-core queues and zero copy real, as SPDK and io_uring
passthrough do on Linux, and as a single address space makes natural:

1. An app opens a file through Files as today. For an unencrypted file Files also
   hands SexDrive a read-only **extent map** for it (disk ranges and block checksums)
   under a lane ticket, and the app a lane capability bound to that ticket.
2. The app's lane lives on its own core: a submission ring and a completion ring
   (`sex_lanes::ring`) shared with the SexDrive worker for that core, which owns that
   core's NVMe queue pair and vector.
3. A read names (file offset, length, buffer). The worker maps it through the extent
   map, checks the buffer lies in memory the app lent it, and DMAs straight into the
   app's buffer through the IOMMU (syscall 90 lending). No Files hop, no copy. The
   app library checks block checksums before returning (sexfs's own check, moved to
   the reader).
4. Writes, metadata, encrypted files and everything else stay on the Files path. A
   write to a file revokes its extent maps first (generation number in the ticket).

Cost: new code in sexfs (extent export), Files (ticket grant/revoke), SexDrive (lane
workers, per-core queues, extent translation), sex-app (lane client), no new kernel
mechanism beyond stage 1. Program loads, media playback and large file reads, the
reads that matter, scale with cores.
