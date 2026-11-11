# Baseline JIT: observed boolean/forward-branch coverage

Implemented behind the existing opt-in flag. Matching Wasm rebuilt; native
execution and speed results remain pending.

The private native `10fdf87d` genuine HTTPS observation retains 26 child cache
keys, including a 26-byte int-argument bitmask/boolean function called 1,109
times. It was not compiled: its logical-not, duplicate/drop and forward
conditional instructions were outside the old straight-line subset. Raw external
bytecode remains only in `sexos-evidence/private-live-embed/real-https-10fdf`.
No third-party function or source is imported by this change.

The verifier admits an entire bounded acyclic CFG before emission. Branches must land on
forward instruction boundaries; each join must agree on stack height and int32
versus boolean kinds. Unsupported/dead instructions, mixed-type joins, malformed
operands, loops and paths falling out of a function fall back. Preserve integer
argument admission, 128-byte functions, 16 stack slots, call polling and all
CodeBuf/W^X/forbidden-byte checks. Arithmetic overflow and zero products retain
existing fallback. Short-circuit branches preserve the selected operand's value
and kind; logical-not/comparisons produce actual JS booleans.

Protocol 2 returns 0=fallback, 1=int32 and 2=boolean through the host import. New
guest code imports `jit_v2` and enables only when `jit_available()==2`. The new
host keeps the old `jit` import as unconditional fallback, so a stale old guest
cannot reinterpret boolean results as integers. New guest/old host has no v2
import and fails closed. Rebuild/pin the Wasm and native wasm2c output together;
this is an engine-private protocol, not a kernel ABI change.

Validation required: focused CFG/operand/type rejection tests, actual x86
truth-table execution and overflow/fallback cases, default-off guest behavior,
real off/on browser results and emitted-entry/seal evidence, then the same
unmodified genuine HTTPS page. A real cache entry is motivation, not proof of
useful executed code or whole-page speedup. GitHub broad CI runs asynchronously.

Targeted macOS verifier tests: five passed in 0.51 seconds including rustc.
This checks bounded admission and rejection, not x86 execution. New host and
browser differential fixtures check boolean types, operand-preserving short
circuiting, signed comparisons, negative zero/NaN and coercion fallback.

Pinned QuickJS 3c9afc9943323ee9c7dbd123c0cd991448f4b6c2 and wasi-sdk 25
rebuilt guest SHA-256
`23c3af9f2fe27c7a59a86a8cd016f25259e933d17e91af023d0b01c2102e20a6`.
The focused real QuickJS wasmi differential test passed (0.23 seconds test
runtime) on macOS ARM. This checks guest import compatibility, interpreter
outputs and fallback, but cannot execute emitted x86 code. The fresh native
wasm2c browser build and actual guest proof are pending. Logs are retained in
`sexos-evidence/jit-forward-boolean/`; GitHub host gates additionally execute
emitted code on x86 Linux and run the unchanged kernel W^X gates.
