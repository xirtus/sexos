# 2026-11-28: NVMe waits end by time, not by a turn count (the flaky "store error=3")

Symptom: full CI on master (and nightly runs since at least 11-27) failed boot gates at
random: `[sexdrive.nvme.wait] cid=1364 ... turns=32768 elapsed_ticks=70..4237
result=Err(TurnLimit)`, then `[sexdrive.dma.quarantine]`, `nvme.flush.err cqe_timeout`, and
every later program load failed (`[shell.program.denied] reason=store error=3`, "busy
program never started", "fetch: no filesystem on this disk").

Cause: `completion_wait` ended a wait after 32,768 pauses even though its deadline is 30 s.
While an IPC message sat in sexdrive's inbox, `sys_wait_self_ticks` returned at once, so
every pause became a bare yield; 32,768 yields took 70 ms on an idle core, less than a host
flush (fsync) takes. The disk was quarantined for a command that was simply slow.

Fix: `MAX_TURNS` is now a backstop (2^22) for a clock that never moves; the deadline ends
waits. The inbox-busy yield lasts one turn, then timed sleeps are tried again.
