## Native QuickJS integration repair

PR #118 merged current master without dropping the iframe/postMessage test or
its recursion and speed tests. All 41 `sex-js` DOM tests pass on the Apple Silicon
host with wasmi. Native trap unwinding uses x86 clang builtins, so non-x86 hosts
explicitly retain wasmi even if `SEXJS_NATIVE=1` is requested.

The build locates Homebrew's wasm-rt header as well as Linux include directories.
Newer wabt versions add a page-size parameter: the runtime accepts only the
standard 64 KiB size and initializes that field; older Linux wabt stays supported.
The linear-memory checks and call-depth limit remain enabled.

This is AOT QuickJS, not a JavaScript bytecode JIT. Runtime code generation is a
separate opt-in topic and must not be inferred from the word native.
