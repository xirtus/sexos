## Baseline JIT acceptance evidence (source ade1890e)

PR #125's first complete candidate, `ade1890e815b246b53ecacfeb769f8112a846351`,
passed all CI checks in run `42820798599` before refreshing against combined
master. It remains experimental and disabled by default. The refreshed
combined-master head must pass its own CI before merge.

Actual x86 host execution, job `129845821760`:
- Four compiler/bounds/emitted-code tests and all 43 DOM/differential tests pass
  with both wasmi and the wasm2c native-AOT engine.
- Warmed 100,000-call long pure-integer arithmetic workload, order off/on/on/off:
  native-AOT interpreter 45.19 / 46.05 ms; runtime JIT 18.43 / 18.40 ms, about
  2.48x. Each measured JIT trial has 99,999 native successes and one conservative
  zero-product bailout. Compilation and engine initialization precede timing.
- Wasmi counterpart: 356.1 / 326.2 ms off versus 129.9 / 130.7 ms on, about 2.6x.
  These are scoped arithmetic results, not general web-page speedups. Short
  functions can still regress; broader coverage/profiling is future work.

Real SexOS QEMU CI jobs `129845821354` (one core) and `129845821525` (four cores)
both passed the generic code-page safety gate and the browser JIT gate:
- Kernel refused hidden key-changing code and a hostile program file;
  reopen/reseal, write-to-code/execute-data faults, CPU faults, retirement and
  subsequent independent runs all behaved correctly.
- The browser's actual QuickJS function bytecode compiled into four native
  functions. 10,068 native successes, three arithmetic bailouts and no refused
  seals; the complete output matched forced interpreter. Code seals were bound
  to each run's target and authenticated task, including relaunched task IDs.
- One-core tiny-function script ticks: 77 off / 72 on; four-core 111 / 76. These
  smoke timings include first compilation and are not a broad performance claim.

Retained exact one-core artifact `sexos-js-jit-1cores` contains the sealed normal
ISO, FFmpeg-capable opt-in Web, hashes, logs and browser result JSON:
- ISO SHA256 `fa177e2bf579fea7d681edb2093869b9d0f7511d4c803a21742ad6dcf9e5d891`.
- Opt-in Web SHA256 `84199156d9421a73d1428d1128536bb054a2e213faf9ff9f0cb59291eb66c477`.
- Default-native, feature-off Web extracted directly from that ISO:
  `b4c0b2d2ece121c24a4a0abbd1dc0d7b4a96471053cbdf9ce7c0a6577c2106dc`.

Local copies live outside Git under `Projects/sexos-evidence/` in the
`pr125-ci-ade1890e-1core` directory. Local one/four-core correctness and kernel
safety gates also pass, but use separately recorded local input hashes. No
physical x17 validation, YouTube playback, complete JS JIT coverage or
Linux-capability parity is inferred from this evidence.
