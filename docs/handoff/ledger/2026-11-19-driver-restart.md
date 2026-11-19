# Crashed and hung drivers restart by themselves (2026-11-19, driver crash recovery)

- **What changed:** the disk driver (Drive: NVMe/AHCI) and the USB driver (xHCI:
  keyboards, mice, tablets, USB disks, USB Ethernet, Bluetooth) now start again by
  themselves after a fault, like Network, Files, Input, Bell, Display and Shell
  already did. Files no longer waits forever on a dead disk driver: the request the
  dead run took is sent again, and the program that asked never sees an error.
- **Hang watch:** a driver that takes a message from its inbox and does not come back
  for its limit (Drive 45 s, past its own 30 s command deadline; USB, Input, Bell,
  Network 10 s) is ended as a fault would end it (`[service.hung]`, crash record
  "no answer (hung)"). Drive also ends itself when a command runs out its deadline
  while serving (`[sexdrive.nvme.stuck]`), so a wedged controller gets a reset.
- **Device reset before the new run:** bus mastering off and IOMMU loans ended (as
  before), then a PCIe function-level reset where the device has one (Drive, USB,
  Bell, Network; never Display), 100 ms settle, then BOOT's config words (BARs,
  command, PCIe device control) and its MSI/MSI-X vectors put back, every one
  masked, and stale interrupt bits dropped (`kernel/src/device_irq.rs`
  `snapshot`/`reset_begin`/`reset_end`).
- **Exactly one answer per call:** while a service is down its calls are refused
  with `ERR_SERVICE_NOT_READY` (checked under the callee's reply-credit lock, so no
  call can land between the old run's drain and the new run). A caller whose call
  the dead run took gets one `sex_pdx::crash_log::PEER_RESTARTED` reply from a
  service that answers every call (Drive, USB), from the reply credits the kernel
  already keeps. Files resends block reads, writes and flushes on it
  (`[sexfiles.block.resend]`); a USB disk sees it as an I/O error and comes back
  through the normal re-plug path.
- **Probe:** test builds (`SEXOS_CRASH_PROBE=1`) take `crash NAME busy` (end it in
  the middle of a request) and `crash NAME hang` (leave it stuck there for the hang
  watch). Spindle has `sum PATH` (size and FNV-1a 64 of a whole file).
- **Gate:** `scripts/driver_restart_gate.py` (CI `crash` job): disk killed mid-write
  and mid-read and hung mid-request, files compared byte for byte with the host; USB
  killed while the tablet moves; Input, Bell and Network killed mid-request; Files
  must never restart.
- **Not yet:** transparent reconnection for Network's clients (open TCP connections
  end with the old run; the next request works), per-request ids in the block
  protocol, hang limits for Files/Shell/Display, and a restart for sexacpi.
