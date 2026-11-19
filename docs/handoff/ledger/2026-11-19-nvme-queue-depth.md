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

How deep a batch gets depends on the drive. A 512 KiB request (`BLOCK_MAX_RUN`) on a
drive with a 128 KiB-or-larger MDTS (the PM9A1 and QEMU) is 4 commands, all in flight
at once. 32 in flight takes smaller commands (a 16 KiB MDTS) or larger requests.
Files still sends one request at a time, so the next gain is from Files keeping more
than one request out, or a larger `BLOCK_MAX_RUN`.

## Tests

- `tests/sexdrive_ioq.rs` (new, in `scripts/sexdrive_dma_host_gate.sh`): the ring
  wrapping with the CID counter wrapping too, the phase flipping at the end of the CQ,
  reverse and interleaved completion order, a full 4-entry SQ holding submissions
  back until SQHD moves, a bogus SQHD ignored, the DMA limit capping commands in
  flight, an error CQE on one command of a batch, foreign and replayed CIDs.
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
