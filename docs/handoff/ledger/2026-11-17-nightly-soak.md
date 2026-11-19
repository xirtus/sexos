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
