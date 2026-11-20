# Nightly soak with fault injection (WS2, part 2)

- `.github/workflows/soak.yml` runs the J2 soak (`scripts/soak_gate.py --inject`) every
  night at 01:30 UTC on a crash-probe image, 2 cores, one 4.5 h segment (a runner job
  may last 6 h). Each round stops one restartable service (Files, Network, Bell,
  Input, Shell, Display) and the session must carry on.
- Each round now also samples the kernel heap and free frames with Swatch; a kernel
  heap that climbs more than `--leak-blocks` (256) live blocks over the first rounds
  fails the run, as free program memory already did.
- `scripts/soak_report.py` writes the step summary: hours, rounds, services stopped,
  and first/last/low/high of free pages, free frames, heap blocks/bytes and key time.
  Nightly reports add up toward the 24 h figure; the weekly 12 h `long-soak.yml`
  (no injection) stays as it is.

## First nightly soak (2026-11-18)

- The 01:30 UTC schedule did not fire on its first night (GitHub skips some new
  schedules); run 44255674618 was started by hand on master at 07:02 UTC.
- Passed: 4.55 h, 133 rounds, 2 cores, a service stopped and restarted every round
  (133 restarts across Bell, Display, Files, Input, Network, Shell).
- Memory was flat to the page and the byte over all 133 rounds: free_pages 51,695,
  free_frames 56,061, kernel heap 1,658 live blocks and 15,941,536 bytes, first round
  to last (low = high for each).
- Rounds slowed from 81 s to 178 s, and key time went from 24 ms to 467 ms (worst
  809 ms). The likely cause is the gate, not SexOS: every 20 ms poll copied the whole
  serial log (several copies per poll, about 27 ms at 50 MB and 90 ms at 150 MB), so the
  host would starve QEMU's vCPU threads as the log grew (log size was not recorded). `SerialText` in
  `scripts/quil_autosave_gate.py` now keeps whole lines as chunks, and polls read
  only the text after their start offset (`since`). Soak fault checks read only the
  text since the last round, and each round prints its key time and log size, so
  the next night shows whether round time stays flat.

## Nights 2 to 4 (2026-11-18 to 2026-11-20)

- Night 2 (master 48280a92): passed, 4.53 h, 126 rounds, memory flat (free_pages
  51,694, heap 1,658 blocks); key time again climbed, 46 to 493 ms (worst 1,635 ms).
- Night 3 (master 128f08c8): stopped at round 32 on a kernel page fault, a supervisor
  fetch at 0 (`rip=0x0 rsp=0x44444493dd90 err=0x10 pd=15`) while Kaleidoscope loaded
  a page after Network was restarted. The same fault, at the same depth below a kernel
  stack top (0x270), hit Moan's gate in #434 (pd 42, no restart, right after heap
  growth), so the cause is not the restart itself. Kernel page faults now print the
  eight words around RSP, and gates print the last 80 serial lines on a guest failure;
  `Task::rearm` also clears a core's handoff slot still naming the task and resets
  its IPC v2 state.
- Branch soak on this fix (run 44626283720): passed, 4.52 h, 136 rounds, no fault,
  memory flat (free_pages 51,317, free_frames 55,683, heap 1,723 blocks / 17,970,584
  bytes, low = high). Key time stayed flat at 21 to 24 ms (one 512 ms outlier) where it
  had climbed past 450 ms: the serial-poll fix works. Rounds still slowed from 84 to
  157 s as the serial log reached 199 MB, because offsets (`len(text())`) and launch
  checks still built the whole log; `SerialText.length()` and a launch check that
  joins the log only when a launch line arrived remove those copies.
