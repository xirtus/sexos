## JavaScript runs as machine code: the native tier (2026-11-10)
- First JIT-plan step for the YouTube /watch page (plan stage J2, "native Wasm", done
  the cheap way): `crates/sex-js/build.rs` translates the existing `quickjs.wasm` to C
  with wasm2c (wabt 1.0.34) and compiles it with clang for `x86_64-sex-app`, so the
  browser runs QuickJS as machine code instead of interpreting it on wasmi.
- Same guest, exports and imports: `sexjs.c`, `prelude.js` and every page op are
  unchanged. Linear memory keeps explicit bounds checks, so scripts stay confined to
  it as under wasmi. Traps unwind with `__builtin_setjmp/longjmp` (`native/rt.c`),
  call depth stops at wasmi's 1024. No libc: `native/include` has the few headers the
  output needs; memory comes from Rust's allocator (`sexjs_rt_realloc`).
- `Script::new` picks native when built (`Script::NATIVE`); `Script::with_engine`
  stays on wasmi. Built for SexOS targets when wasm2c is on PATH (`SEXJS_NATIVE=0`
  turns it off); on the host with `SEXJS_NATIVE=1`. CI installs `wabt`.
- Speed (host): sex-js DOM suite 13.2 s -> 2.0 s single-threaded; fib(24) + 20k
  string sort 143 ms -> 25 ms (about 6x).
- Not a JavaScript JIT yet: QuickJS still interprets its bytecode, now at native
  speed. Next: JS-level JIT (V8 Sparkplug in its own organ, needs the `sexos` cloud
  env), or a baseline JIT for QuickJS bytecode through `CodeBuf`.
