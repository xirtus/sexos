## Explicitly disable wasm runtime TLS in SexOS browser builds

The newer wabt header selects `__thread` even in C99 mode. Native QuickJS had
assumed C99 meant no thread-local storage. On the local candidate, a real
browser load faulted at address zero before script initialization: disassembly
at the recorded RIP showed `movq %fs:0,%rax`, and the Web ELF had a TLS segment.

The build now explicitly removes the thread-local storage qualifier from its
private wasm-rt header copy for SexOS targets. Each browser engine runs on one
thread; its globals belong to its own program, while nested traps still restore
the outer catcher. Host builds keep wabt's thread-local trap state for concurrent
tests. Header/source rebuild tracking is unchanged.

This fixes the native runtime before any JavaScript-level JIT executes; it is
not JIT evidence. The opt-in Web release rebuild passes the static key-byte
scan and has no TLS segment. Guest execution is being rerun separately.
