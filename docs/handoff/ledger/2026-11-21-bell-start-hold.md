# Bell: a new stream waits up to 200 ms for its first whole period (2026-11-21)

- `sound_gate.py --card hda` failed on several PRs during the x17 PR sweep with the
  live tone as 50 to 120 ms, an 80 ms gap, then the rest (e.g. `[(19.62, 0.05),
  (19.75, 1.95)]`): the total length was right, so nothing was lost; Bell's start
  hold (`START_HOLD_TICKS`, 50 ms, from 2026-11-20) ran out before Qupid's first
  whole period arrived under a loaded CI runner, and the partial period went out
  padded with silence.
- The hold is now 200 ms. Streams that fill a period sooner start as before; only a
  sound shorter than one period (about 43 ms) can start up to 200 ms late.
