# Limits and performance: state on 2026-11-10, and what comes next

Owner thread: "Remove limits to reach Linux-level capability". Full detail:
docs/handoff/LIMITS_AND_PERFORMANCE_PLAN_2026_11_08.md (head-to-head tables at the end).

Merged today: #89 (latency classes, Linux head-to-head), #114 (Files stat cache),
#119 (boot: kernel entry TSC, 1 s loader countdown, batched boot log drawing).

Against Linux 6.8 on the same QEMU (TCG, 1 core, no PKU), SexOS is now ahead on
system calls, IPC, memory, file read/write, 4 KiB reads, stat (2.7k vs 40k cycles) and
power-on to first program (6.81 G vs 7.46 G cycles).

## Next, in order
1. **Serial log trim.** The kernel writes ~263 KB of log before the first program.
   COM1 is probed and skipped when absent, and the boot screen now draws in batches.
   But a machine with a real 115200-baud UART would spend ~23 s writing that log.
   Plan: give `serial_println!` a level; send bulk/trace lines only to the in-memory
   log (`console::LOG`, kept by the crash log); keep on the UART only the lines gates
   and the x17 checklist match (boot proofs, `[wifi.*]`, `[sexnet.*]`, `[crash.*]`,
   `[files.*]`, `[sched.latency]`, panics). Check every gate regex before demoting a
   tag: `git grep -h -o '\\\\\[[a-z_.]*' scripts/ | sort -u` lists the tags gates use.
2. Stage 6b: message in registers on wake (IPC), services per core.
3. Per-core NVMe queues: only after a benchmark that reads uncached data (the 4 KiB
   bench is served by Files' cache).
4. Stage 5b (more program rows / organ keys), 7b (deadline class, PCID).

## Gotchas
- ABI pins: `scripts/update_abi_hashes.sh` (spec/abi_inputs.sha256); on a conflict take
  master's side, then regenerate.
- Local builds need `musl-dev`, a fresh `scripts/build_ffmpeg.sh` after FFmpeg changes,
  and `SEXOS_CRASH_PROBE=1` for crash_screen_gate's later steps.
- Never rebuild the ISO while gates run. Measure with `SEXOS_GATE_CPU=max,-pku,+rdseed`
  (TCG's wrpkru costs ~3k cycles).
