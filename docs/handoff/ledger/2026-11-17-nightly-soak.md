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
