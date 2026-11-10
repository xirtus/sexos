## Native QuickJS host C11 headers

The new x86 host baseline-JIT gate caught a native-tier build failure with the
runner's newer wabt: its C11 header includes `threads.h`, while the build removed
all system include directories. The SexOS C99 build was unaffected.

`-nostdinc` now applies only to the bare-metal SexOS compilation. Host builds
retain system C11 headers, with the private runtime headers ahead of them; the
runtime's allocation and trap semantics are unchanged. SexOS still admits only
private/clang headers and explicitly disables wasm runtime TLS.

The wasmi half of the same x86 CI job passed all 43 DOM/differential tests and
the emitted-code arithmetic boundary tests. Its warmed long-arithmetic workload
measured 275.5/288.5 ms with the interpreter versus 113.4/107.2 ms with the JIT,
roughly 2.5x for that controlled workload. Native-AOT and broader page speedups
remain separate measurements; tiny-function guest smoke timings are noisy.
