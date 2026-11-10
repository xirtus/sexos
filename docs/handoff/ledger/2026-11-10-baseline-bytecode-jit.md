## Experimental real QuickJS bytecode JIT

The opt-in `baseline-jit` Cargo feature on `sex-js`/`web` adds runtime x86-64
compilation of hot QuickJS-NG v0.10.1 function bytecode. Default builds stay on
the existing interpreter/AOT tiers. `run +net web URL --no-jit` disables the new
tier in an experimental browser build for comparison.

This deliberately changes the handoff's route A: QuickJS stays inside its Wasm
linear-memory sandbox (wasmi or wasm2c). A guest call hook passes **bytecode**,
not page source patterns, to the Rust host. After 32 eligible calls, a complete
validated function is compiled. The tiny compiler accepts int32 constants and
arguments, add/subtract/multiply, bitwise and/or/xor and a final return. It
accepts no calls, loops, heap accesses, closures, local variables or side effects.
This is a first real baseline tier, not a YouTube `/watch` solution.

Safety:
- All guest offsets, JSValue int tags, argument bounds, stack effects and the
  complete final return are checked. Arguments are copied to a fixed host array.
- Generated code has no guest pointers, indirect calls or arbitrary branches.
  Overflow and every zero product bail out before writing a result, preserving
  double arithmetic and negative zero via QuickJS. Other types/coercions and
  unsupported bytecode execute in QuickJS before any effect.
- QuickJS still polls interrupts before every call; generated functions have at
  most 128 bytecode bytes and no loops. The existing timeout/recursion gates run.
- Each Script owns at most 64 cached functions; no global executable cache.
  SexOS code uses its owning program/key's CodeBuf: writable then sealed RX,
  reopened on drop. Kernel scan/seal failure falls back. No kernel growth.
- Checked short immediates use direct loads; dangerous payloads are constructed
  byte by byte to avoid hidden WRPKRU/XRSTOR/
  XRSTORS; relative branches are padded and rescanned. The kernel checks again.

Gates:
- `cargo test -p sex-js --features baseline-jit`: malformed bytecode/linear offsets,
  dangerous constants, interpreter differential (overflow, -0, strings, booleans,
  undefined, floats, BigInt, throwing coercions and side-effectful functions),
  existing DOM, recursion and script deadline tests. Linux x86 additionally
  executes emitted functions over boundary-value operand pairs. CI runs both
  wasmi and wasm2c with the feature and records warmed, repeated/inverted-order
  100k-call timings for a longer arithmetic function. The browser logs init
  and script-only ticks separately.
- `scripts/build_web_jit.sh OUTPUT`: separate opt-in browser with static key-byte scan.
- `scripts/program_web_jit_gate.py --iso EXACT_HEAD.iso --source-disk BLANK.img
  --program OUTPUT --out EVIDENCE`: same real page with JIT forced off/on, exact
  result, actual compilation/execution counters, domain-specific kernel seal,
  graceful close/relaunch. Hashes bind ISO and program. Dedicated CI jobs run
  this and the kernel JIT gate on both one and four cores.
- Existing `scripts/program_jit_gate.py` tests kernel write-to-code/execute-data
  faults, blocked code and per-run crash isolation. This new tier does not
  weaken those mechanisms.

Local Apple Silicon validation: 43 DOM and three compiler/bounds tests pass on
wasmi fallback; this host cannot execute x86 code. Native execution/performance
and guest readiness must be reported from x86 CI/QEMU evidence, not inferred
from local compiler tests. Rebuild uses pinned QuickJS source and wasi-sdk 25.

First local one-core smoke passed after the native-runtime TLS repair: four
functions compiled, 10,068 native successes, three arithmetic guard bailouts,
zero refused seals; output matched forced interpreter and kernel seals belonged
to the authenticated run task. Proof: `pr125-local-1core/browser-final/result.json`
in the external evidence directory. ISO SHA256
`71754aac895f92a4f29813f04b1d1b4e49e0f8b078c275ec26f7e87d5ebbbdb1`,
program SHA256 `276b88ab5d5b9149c3f11e854157c0266a87626272783f57f0bc3b73b8d4d2ae`.
That first tiny-function smoke was slower in script-only ticks (46 off, 108 on),
with strongly different initialization costs (898 vs 2033). It proves execution
and parity, not speedup. Safe constant loads and explicit timing were then
improved; useful speedup remains a separate measured acceptance gate.
