# 2026-11-19: NVMe batches, up to 32 commands in flight

SexDrive used to send at most two NVMe commands at once, on a 16-entry I/O queue.
Now a request longer than one command goes to the drive as one batch:

- The I/O queue pair has 64 entries, or fewer when CAP.MQES says so
  (`ioq::queue_entries`). 64 SQEs are still exactly one 4 KiB page. Boot log:
  `[sexdrive.nvme.queue] qid=1 entries=64 mqes=... batch_max=32 prp_slots=32`.
- `nvme_batch` writes every SQE that fits, rings the doorbell once, reaps completions
  by CID in whatever order the drive finishes them (and every further CQE already
  posted before ringing the CQ doorbell), and refills freed slots until the request
  is done. Up to `ioq::MAX_IN_FLIGHT` (32) commands are out at once. Long writes use
  it too (before, they went out one command after another). It replaces the
  two-command `nvme_read_queued`. RAID volumes and AHCI still take the pieces one at
  a time. Sampled log: `[sexdrive.nvme.batch] n= op= bytes= commands= in_flight_max=
  doorbells= limit= dma=loan|bounce`.
- DMA custody follows `dma_transfer.rs`: each command owns one lease from its SQE
  until the CQE with its own CID and phase. Reads that land in Files' loaned block
  buffer (IOMMU) need only a PRP list per command: 32 lists of 256 bytes are carved
  from two pool pages (`[sexdrive.dma.prp_slots]`). Without a loan, each command
  needs a 128 KiB bounce. The read and write leases plus up to two spare leases,
  taken from whatever the 512 KiB pool has left after probing, are the bounces
  (`[sexdrive.dma.bounce] buffers=`). So without an IOMMU the pool caps a batch at
  2 to 4 commands of 128 KiB.
- If one command fails, no new commands go out, the rest are still reaped (their
  buffers are only free after their own CQEs), and then the queue stops, the same
  as for one command (`[sexdrive.nvme.batch.stopped]`). If a CQE never comes,
  everything still out is quarantined and the loan ends first.

## Runs of 4 MiB, so a batch fills

A batch can only be as deep as the request is long, so `sex_pdx::BLOCK_MAX_RUN` (the
Files/SexDrive shared block buffer, region 4) grows from 512 KiB to 4 MiB: 32
commands of 128 KiB, a whole batch on a drive with a 128 KiB MDTS (the PM9A1, and
QEMU). Before this, a 512 KiB run was 4 commands. What allows the size:

- BOOT backs the region with one physically contiguous block (`try_backing`, buddy
  order 10 for 4 MiB) and maps it into both domains with 4 KiB pages. That costs
  3.5 MiB more memory at boot.
- The IOMMU loan of the whole buffer is 1,024 pages, well inside the 128 MiB lend
  window. The kernel needs 4 KiB leaves to lend, which the region has.
- SexDrive's own DMA pool is still 512 KiB. With the loan, a run needs only the 32
  PRP list slots (8 KiB). Without the loan, a run goes through the 2 to 4 bounces,
  refilled as they free up: more round trips through the batch, still one Files call.
- Files needs no change. It sizes everything from the region it is given
  (`diskfs_bridge_buf_len`): sexfs device runs, readahead (held in the buffer
  itself), program-store loads, FAT/NTFS volume reads and the installer's copies.
- sexfs had its own 512 KiB caps. Readahead and side-by-side writes now go up to
  `LONG_RUN` = 1,024 blocks (4 MiB). The per-run checksum array, `RUN_MAX` = 128,
  is unchanged: it covers only what a read hands over, which a 64 KiB request
  bounds anyway.

Files still sends one request at a time. Keeping several outstanding would need
the block protocol and the Files service loop to change (and open PR #444 is
editing `fs_service.rs`), so that option was not taken. Readahead and large runs
reach the same depth with one call.

Found while testing: sexfs places copy-on-write tree blocks right after each data
run it writes. A file written in 512 KiB pieces therefore sits in 512 KiB extents,
and readahead stops at each extent's end: a 4 MiB file read sequentially takes 9
device runs, not 2. Files commits after every 63 KiB request, so files written
through the request region are probably in extents that small. This was reasoned
from code and not measured on a real image. Allocating tree blocks away from data,
or allocating data late, would let readahead and the 4 MiB runs reach ordinary
files. Until then, the big runs mostly help program loads, foreign volumes and
installs.

ABI: `BLOCK_MAX_RUN` 512 KiB -> 4 MiB (region 4 size; `spec/abi_inputs.sha256`
re-pinned; `check_self_wait.py` admits the new sex-pdx wait fingerprint with a
dated entry; `check_memory_authority_log.py` expects `region_id=4 length=4194304`).

## Tests

- `tests/sexdrive_ioq.rs` (new, in `scripts/sexdrive_dma_host_gate.sh`): the ring
  wrapping with the CID counter wrapping too, the phase flipping at the end of the CQ,
  reverse and interleaved completion order, a full 4-entry SQ holding submissions
  back until SQHD moves, a bogus SQHD ignored, the DMA limit capping commands in
  flight, an error CQE on one command of a batch, foreign and replayed CIDs.
- `tests/sexfs.rs`: `a_four_mib_buffer_reads_a_sequential_file_in_two_commands`.
- `tests/sexdrive_dma_transfer.rs` compiles the production `nvme_batch`: one doorbell
  for 4 bounced commands that complete in any order; two bounces refilled in the order
  they free up; 32 zero-copy commands in flight; one PRP list per slot; an error on one
  command (the rest reaped, nothing quarantined, queue stopped); a timeout (only the
  commands still out are quarantined, nothing resubmitted, the loan ends); a batched
  write copies each piece into its own bounce before the doorbell.

## Next: one queue pair per core

Not done in this pass. sexdrive is one task that serves Files' requests one at a
time, so more queues add no parallelism yet. The plan, once there are several
submitters (per-core sexdrive workers, or Files keeping several requests out):

1. Set Features, Number of Queues (FID 0x07) with NSQR = NCQR = online cores - 1,
   read the granted counts from CQE DW0, and use min(cores, granted).
2. Create one CQ/SQ pair per core (QID 1..n), each with its own `ioq::Ring` and
   `NVME_IO_STATE`-like record and its own PRP slots and bounces (the 512 KiB pool
   does not hold four bounces per core, so this needs the IOMMU loan or a larger
   BOOT DMA grant).
3. Interrupts: the DRIVE manifest (`crates/sex-hwdb/src/manifest.rs`) grants
   `vectors: 1`, so every CQ would start on IV=0 and a wake would have to scan all
   CQs. Per-core vectors need the manifest and BOOT's MSI-X programming to hand out
   one vector per queue, steered to that core.
