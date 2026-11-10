## Handoff: true JavaScript JIT for the YouTube /watch page (2026-11-10)
State on master after PR #118:
- sex-js runs QuickJS as machine code (wasm2c of `quickjs.wasm`, `crates/sex-js/build.rs`,
  `native/rt.c`). About 6x faster than wasmi on the host. `Script::NATIVE` says which tier is in use.
- Kernel JIT support exists (J1-J5): `SYS_PROGRAM_CODE` 82 and `sex_app::code::CodeBuf`
  (W^X seal/open; sealed code may not hold WRPKRU `0f 01 ef`, XRSTOR `0f ae /5` or
  XRSTORS `0f c7 /3` at ANY byte offset), threads (83), per-program faults, TLS,
  pthreads, and libc++ (`scripts/build_libcxx.sh`). See `docs/handoff/V8_HANDOFF_2026_11_09.md`.
- CI runs `scripts/program_web_js_gate.py` in QEMU. No other CI step boots the browser.

Next steps, in order:
1. Run every browser gate on SexOS with the native tier and fix what breaks: `program_web_*_gate.py`
   (canvas, webgl, three, animation, frames, mse, web_apis, pointer). The pointer gate
   also fails on master. Measure pinball fps and three.js load time against wasmi.
2. Make QuickJS itself faster (no JIT yet): build with `-O3`, and check the call depth
   (1024) and the 2 MiB web stack on deep pages.
3. True JIT, route A (Rust-first, small): a baseline JIT for QuickJS bytecode.
   - QuickJS-NG must then run natively, not through wasm2c. Compile `quickjs.c` with clang for
     `x86_64-sex-app`, like `apps/cpp`, against sex-libc. Keep a separate PKU key/organ for the engine heap.
   - Emit x86-64 per function for hot bytecode (call-threaded first, then inline caches for get/put_field) into `CodeBuf`.
   - Re-encode any immediate or displacement that would form the key bytes, and pad when seal reports an offset.
4. True JIT, route B (largest win): V8 Sparkplug/Maglev in its own organ.
   - Follow `V8_HANDOFF_2026_11_09.md`.
   - Needs the `sexos` cloud environment, because googlesource is blocked in Default.
   - Needs a post-link key-byte rewrite pass.
5. YouTube: iframes landed (#117). Next, load the real embed in a frame on a real https page,
   then the /watch page (~10 MB Polymer JS), which needs route A or B.
Rules: kernel line budget (docs/DESIGN_GUARDRAILS.md); no UA or client spoofing;
ledger entries in docs/handoff/ledger/; `abi_version_hash` conflicts -> drop the line,
run scripts/update_abi_hashes.sh; merge own PRs when CI is green.
