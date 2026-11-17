# Bigger storage runs, two NVMe commands in flight, sequential readahead (#307)

Files and SexDrive now move large reads in fewer, bigger trips:

- The Files/SexDrive shared block buffer grows from 128 KiB to 512 KiB
  (`sex_pdx::BLOCK_MAX_RUN`). One device command still carries at most 128 KiB
  (`sex_pdx::BLOCK_CMD_RUN`, one PRP list page).
- SexDrive splits a long read into 128 KiB commands and keeps two in flight at
  once (read lease + write lease as bounces, or the IOMMU loan): both SQEs go out
  with one doorbell and the CQEs are accepted in either order
  (`nvme_read_queued`, `completion_wait::wait_for`). RAID and AHCI take the
  pieces one after another; long writes go out as consecutive 128 KiB commands.
- sexfs notices a sequential reader (a read that starts where the last data run of
  the same file ended) and asks the device for the file's next contiguous blocks in
  the same command (`Device::read_data_ahead`). Files keeps them (`fs_service::ahead`)
  until the reader asks, any write, or a new mount; sexfs still checks every block's
  checksum when it is handed over. Readahead never reads past the file, so
  `bench read-large` still sees every byte come off the device once per pass.
- The program store already reads in buffer-sized runs, so program loads now go in
  512 KiB runs with two commands in flight (#305 "one pipelined run").

ABI: `BLOCK_MAX_RUN` 128 KiB -> 512 KiB (shared region 4 size), new `BLOCK_CMD_RUN`.
No kernel file changed (the kernel sizes the region from the sex-pdx constant).

## Verification

- `scripts/sexfs_host_gate.sh`: new `sequential_reads_fetch_ahead_in_one_command`
  (4 MiB in 64 KiB reads takes <= 16 device commands, exact payload bytes, writes
  drop held blocks, random reads get no readahead).
- `scripts/sexdrive_dma_host_gate.sh`: new
  `queued_read_keeps_two_commands_in_flight_and_accepts_completions_in_either_order`.
- CI `Large reads must reach the device` gives the before/after numbers (PR body).
