# Build the real C demo with current Clang

Supplying musl headers exposed a second local build-input problem: Homebrew
clang 23.1.5 rejects Chello's `at_exit_ran` under its existing `-Wall -Werror`
flags because the file-local variable is assigned but never read. Apple clang
21 compiled that source, which allowed the qualified real-C UEFI run to proceed.

Remove only the unused variable and its unobserved assignment. The registered
atexit callback still writes the same `[chello.atexit]` marker; every test and
score remains unchanged. No warning is disabled, and no C capability is stubbed.

Validation uses the exact production C17/x86/SSE2/no-red-zone/PIC compile flags
and the verified musl 1.2.5 headers. The original source fails clang 23 with the
expected unused-global diagnostic; the corrected source passes with `-Werror`.
Both sources pass Apple clang 21 and produce byte-identical optimized objects
(SHA-256 `38d4c52ae647c1ec024405a86afa8f0dd7cde83118dfd0f1b55609a43c8488c8`).
Compiler versions, commands, source/object hashes and retained diagnostics are in
`Projects/sexos-evidence/chello-clang23/`. The existing UEFI C21/21 and C sockets
23/23 proof uses the unchanged Apple-generated C code. This compile check does
not claim a separate clang-23 guest runtime run. Current-head CI remains required.
