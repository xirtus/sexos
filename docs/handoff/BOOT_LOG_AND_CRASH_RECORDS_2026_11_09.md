# Boot log on screen, stop screen, crash records, restartable services, fuzzing (2026-11-09)

Workstream C of `LINUX_AND_DESKTOP_PARITY_REPORT_2026_11_09.md` section 6. Goal: next
week's bare-metal test must not depend on a serial port. A PC that hangs, stops or loses a
service has to say so on its own screen, and the next start has to say what happened.

## What a person sees

1. **Boot log.** From the kernel's first instruction until Display takes the framebuffer,
   every line the kernel logs is drawn on the screen (5x7 font, scaled to about 45 rows on
   any resolution, 2x at 1280x800). A title bar says that the last line is where a hang
   stopped. The log scrolls by half screens so a long boot costs one redraw per ~20 lines.
2. **Stop screen.** When the kernel cannot go on (a panic, a page fault in kernel code, a
   double fault), or Display or Shell end, the kernel takes the framebuffer back from
   whoever holds it and paints a red screen: "SexOS stopped", the reason, core and domain,
   and the newest lines of the log that fit. A footer counts down 30 s and then restarts
   the machine (keyboard controller reset, then the chipset's 0xCF9, then a triple fault);
   a key on a PS/2 port (or the firmware's USB legacy emulation) restarts at once.
3. **Next start.** If the previous start stopped, the boot log pins an amber note at the
   top for 4 s: the reason, where, how long it had run, and the last five log lines.
4. **Spindle `crashes`.** Lists every crash record, newest first: this start's and the ones
   Files kept on disk from earlier starts.

## How it works

- `kernel/src/console.rs` holds a 64 KiB log ring (every `serial_println!` and every
  service's serial record go there, with or without a serial port) and the framebuffer
  console. It never allocates. The stop path (`console::stop`) masks interrupts, sends an
  NMI to every other core (`apic::stop_others`; the NMI handler halts while stopping),
  opens every PKU key (the framebuffer page is Display's), writes to serial without the
  lock, records the stop and draws. `serial::_print` tees into the console.
- `kernel/src/crash_log.rs` keeps up to 64 records for this start (`sex_pdx::crash_log`,
  one 512-byte sector each, checksummed). The kernel adds one when a service or program
  faults (CPU exceptions and terminal page faults), when a service is started again or
  given up on, and when the kernel stops.
- **Kept across a restart.** The stop writes its record and the last 4 KiB of the log to
  8 KiB at physical 48 MiB (`PSTORE_PHYS`), which BOOT keeps out of both the boot frame
  allocator and the buddy allocator (the region is split in `memory::manager::init`). The
  next start reads it before anything can reuse it, shows it, adds it to the ring and
  clears it. RAM keeps its contents across a warm restart (QEMU with OVMF: verified); a
  power cut loses it, which is why the stop screen restarts by itself.
- **Kept on disk.** Files owns the reserved ring at LBA 64..127 (64 sectors; free in the
  fixed DiskFS area, checked at compile time against every other fixed range; SexDrive's
  write guard admits it). At start Files reads the ring and hands earlier starts' records
  to the kernel (`OP_ADOPT`), then writes every kernel record not yet on disk. The kernel
  tells Files about new records with a message from caller 0 (`OP_NOTICE`) sent from the
  timer, as it does for app exits.
- **Restart in place.** `service_restart` now covers Network, Files, Input and Bell (was
  Network and Files). A service started again asks `OP_RESTARTS` and skips its once-only
  start steps: Input does not wait for Shell's ready again, Bell does not repeat its
  paging check. Display and Shell hold the desktop's whole state and cannot yet start
  again in place; if either ends, the kernel stops the machine with the stop screen and a
  record instead of leaving a frozen screen.
- **Fuzzing.** `apps/fuzz` (`run fuzz [CALLS] [SEED]`) makes seeded random syscalls (every
  number up to 100 except the few that end, block or reshape the caller) and pdx calls
  (half to slots a program holds: Storage, Display, Shell, Bell, Net, Files' region route;
  half to any slot), with edge values, kernel and unmapped addresses, pointers that cross
  its own buffer's end, and noise. The gate runs three seeds x 20000 calls and requires no
  kernel stop, no service fault, the fuzzer's own report, a normal close and a working
  Spindle afterwards.

## ABI changes (abi_version_hash recomputed)

- New syscall **84 `SYS_CRASH_LOG`** (`sex_pdx::crash_log`): `OP_READ` and `OP_STATUS`
  (Files, Shell, any Spindle), `OP_ADOPT` (Files only; refuses this start's own records, puts history in front of them and returns `FULL` rather than push out a record not yet on disk),
  `OP_RESTARTS` (any service, about itself), `OP_PROBE_FAULT` (test builds only, Spindle
  only).
- New kernel message to Files: `crash_log::OP_NOTICE` (0x0C5A_0001), caller 0.
- New crate module `sex_pdx::crash_log` (added to `ABI_INPUTS`): record format, disk ring
  location `DISK_BASE_LBA = 64`, `DISK_SECTORS = 64`.
- Service serial records no longer write to an absent serial port (they go to the log).

## Test builds

`SEXOS_CRASH_PROBE=1` adds Spindle's `crash kernel` / `crash NAME` (ends that service at
its next syscall, as a fault would; the target is woken so it makes one).
`SEXOS_STOP_RESTART_SECONDS=N` shortens the stop countdown.

## Gates

- `scripts/crash_screen_gate.py` (probe build; CI runs it last): start 1 checks the boot
  log pixels, ends Network, Input and Bell (each restarts; keys still reach Spindle after
  Input's restart, `beep` works after Bell's), lists `crashes`, stops the kernel and checks
  the stop screen pixels and the self-restart; start 2 (warm) finds the kept stop, shows
  the amber note, writes it to disk; start 3 (cold, RAM gone) reads all records back from
  disk and lists them, then ends Shell and checks the machine stops instead of freezing.
- `scripts/syscall_fuzz_gate.py`: as above.
- `scripts/crash_log_host_gate.sh`: record format and ring slot choice on the host.

## Cost

The boot log costs about 1 s of boot under QEMU TCG (glyphs written a row at a time; one
redraw per half screen). On real hardware the framebuffer is write-combined and the cost
is a few milliseconds.

## Not done yet (next steps)

1. **Display and Shell restart in place.** Needs a restart epoch in the Display and Shell
   protocols: a restarted Display gets its framebuffer handoff and display lease again
   (the kernel would re-send `DisplayPrimaryFramebuffer` and re-seed `ext_init`), then
   asks Shell and every client to present again; a restarted Shell asks Display for the
   surfaces that exist and rebuilds its window list from the app catalog. Until then the
   machine restarts (with a record) instead of freezing.
2. USB keyboards on the stop screen when the firmware has no legacy emulation (needs a
   polled xHCI path in the kernel, or a hardware watchdog restart).
3. A crash record for hangs: a watchdog on core 0's timer that stops the machine when no
   task has run for N seconds, and a "Display did not start" check after boot.
4. Fuzzing deeper: per-service request fuzzers (Files, Display, Shell protocols with
   valid shapes and bad values), longer CI runs on a schedule, multi-core runs.
