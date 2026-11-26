# 2026-11-25 `tether sweep`: unattended Wi-Fi test matrix

For the x17 Wi-Fi tether stall (TCP stops after 35-87 KB), one run now tries many
combinations instead of one image per guess.

- Spindle `tether sweep HOST[:PORT] [ROUNDS 1-50] [SHAPE]` (default 4 rounds = 128
  trials). Each trial sets `wifi ba on|off` and `wifi replay on|off`, connects to
  `tether_host.py`, moves data in one shape and ends as `pass`, `stall` (nothing moved
  for 20 s), `timeout` (240 s) or the network's failure ("network error", "could not
  reach ..."). Then `wifi` logs the link counters, and the result plus those counters
  are appended to `/media/Ventoy/wifi-sweep.txt` (rewritten after every trial). The end
  adds a passes/runs table per shape and switch setting. `tether off` stops it.
- Shapes (`SHAPES` in apps/spindle/src/tether_command.rs): bulk 512 KiB in 4 KiB sends,
  mid 1 KiB, small 128 B, paced (100 ms between sends), trickle, idle45 (45 s quiet after
  connecting), down (512 KiB from the Mac to SexOS) and long (2 MiB).
- After a failed trial the next one waits 15 s for the link to recover.
- `scripts/tether_host.py`: a session whose first line is `SEXOS-SWEEP ... down=N` gets N
  bytes back; sweep sessions keep only the header line and the byte count (no filler
  in the logs).
- Gate: `scripts/tether_gate.py` step 7 runs `tether sweep 10.0.2.2:PORT 1 down` (four
  trials, all pass); host unit test `test_sweep_down`.
