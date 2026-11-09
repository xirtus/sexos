# JIT engines and a Chromium-class browser for SexOS: report and plan (2026-11-09)

Andreas approved step 3 of the browser's JavaScript speed plan: give the kernel what a
JIT engine needs, port a JIT engine, and aim the Web browser at Chromium-class
capability. He wants SexOS to do better than Linux and Chromium where its design
allows, and a bare-metal test next week. This report compares the options, says where
SexOS can do better, and sets the order of work. Stage J1 is already implemented in the
same pull request (see "Done in this PR").

Related threads: "Speed up browser games" owns QuickJS speed (wasmi tail-call dispatch,
then wasm2c); "Remove limits" owns kernel memory (arenas, unlimited rows, PCID).

## 1. Where we are

- **JavaScript today** is QuickJS-NG compiled to Wasm and interpreted by wasmi, inside
  the `web` program (`crates/sex-js`). That is two interpreters stacked. On the host
  pinball runs about 480 fps; under QEMU's emulator (no KVM, 20 to 50 times slower than
  hardware) it runs 8 to 11 fps.
- **Programs** are single-task Rust `no_std` ELFs on one PKU key per row, in a private
  arena of up to 256 GiB whose page tables appear on first use. Pages come from
  `SYS_PROGRAM_MEMORY` (81). The loader enforces W^X on the file's segments.
- **Missing for a JIT engine:** pages a program can turn into code (now done, J1),
  threads, faults a program can handle itself, a C/C++ runtime, and sockets/files with
  POSIX-like shapes for the large C++ code bases.

## 2. Engine options

| Engine | Speed vs QuickJS | Size | Port surface | Fit |
|---|---|---|---|---|
| **V8** (Chrome, Node, Deno) | 10-50x (Sparkplug, Maglev, TurboFan) | ~2M lines C++, GN build | `src/base/platform/platform-*.cc` (~2k lines: memory, threads, time, mutex); runs with `--single-threaded` and without signal handlers | **Best.** Same engine as Chromium, so a later Chromium port reuses it. Has a Fuchsia port (non-POSIX microkernel) and a Starboard port to copy from. `rusty_v8` gives Rust bindings. |
| SpiderMonkey (Firefox, Servo) | 10-40x (Baseline, Warp) | ~1M lines C++, mach build | NSPR-free in recent versions, still POSIX-heavy | Good if we go the Servo route. `mozjs` crate gives Rust bindings. |
| JavaScriptCore (Safari) | 10-40x (LLInt, Baseline, DFG, FTL) | ~600k lines C++, CMake | WTF platform layer; PlayStation port proves non-Linux | Its "separated W^X heap" was built for Apple's per-thread JIT locks, close to our PKU model. Ties us to WebKit for the browser. |
| wasmtime + Cranelift (Rust) | Wasm at near-native speed | Rust, no_std runtime | `custom-virtual-memory` platform hooks | Not a JS engine, but it is the fastest real JIT we can ship: QuickJS-on-Wasm and page WebAssembly both speed up. On no_std, Cranelift runs ahead of time on the host; the device loads native code (needs J1). |
| Own baseline JIT for QuickJS bytecode | 2-4x | Small | None beyond J1 | Possible, but V8 makes it throwaway work. Not recommended. |

**Recommendation: V8.** It is the fastest, it is what Chromium and Helium run, and its
platform layer is small and well-trodden (Fuchsia, Starboard, Windows, Mac, iOS).
Bring it up in stages: `--jitless` (Ignition interpreter only, needs no executable
pages), then Sparkplug (baseline JIT, needs J1), then Maglev and TurboFan
(optimising, need threads for background compilation, or run on the main thread).

wasmtime is a parallel short path: once J1 exists, the Wasm runner and the web
program can run precompiled native code. It overlaps with the games thread's wasm2c
step; I will offer it to that thread rather than duplicate it.

## 3. Browser options

| Route | What we get | Cost | Notes |
|---|---|---|---|
| **A. Our engine (sex-html + sex-web) + V8** | Our layout, our painter, V8 for JS | Smallest; reuses everything built so far | Keeps every SexOS advantage below. Limits: our CSS/DOM completeness is years behind Blink. |
| **B. Chromium port (content_shell), Helium-style patch set on top** | Full Blink, V8, Skia, media, devtools | Largest: ~35M lines, needs a POSIX layer good enough for `base/`, an Ozone platform for windows/input, threads, sockets, shared memory | Precedent: Fuchsia (non-POSIX microkernel) is an upstream Chromium platform. Helium is ungoogled-chromium plus patches, so "Helium-style" is a patch set we keep over our port. Cobalt (YouTube's Chromium for TVs and consoles) uses the Starboard layer, which since Starboard 16 is POSIX-based, so it is a smaller reference port, not a shortcut. |
| C. Servo (Rust) + SpiderMonkey | Rust engine, modern CSS (Stylo), WebRender | Medium: needs Rust `std` for SexOS, threads, SpiderMonkey | Most Rust-native. Thinner web compatibility than Blink. |
| D. Ladybird | New C++ engine | Medium | No JIT today. Not a fit for speed. |

**Recommendation: A now, B as the compatibility browser.** A gives fast pages within
weeks of V8 landing and keeps the SexOS-only advantages. B is the long-term "runs
every site" browser; it needs the same base as V8 plus a POSIX layer, so every step
of A is on B's path. C stays an option if Rust `std` arrives early.

## 4. Where SexOS can do better than Linux and Chromium

1. **Code that cannot unlock memory.** Linux lets any code run WRPKRU, so PKU there is
   a bug guard, not a boundary. SexOS now refuses program files and sealed JIT code
   that contain WRPKRU or XRSTOR at any byte offset (J1). A JIT bug or a sprayed
   constant cannot turn into an instruction that opens other keys.
2. **Executable memory as a checked transition, not `mprotect`.** Sealing goes through
   one kernel call that checks the bytes, flips W to X on every core, and records the
   change in the run's page manifest, so retire and rebirth still verify every leaf.
3. **Out-of-domain JIT for almost nothing.** Chromium's renderer compiles its own code,
   so a renderer exploit can write code. In a single address space, a compiler organ
   can write code that only the renderer can execute, with a PKRU switch (tens of
   cycles on hardware) instead of a process switch. Pointers mean the same in both, so
   nothing is copied or relocated. This is Edge's out-of-process JIT, made cheap.
4. **Site isolation without processes.** Chromium spends a process (tens of MB, a
   page table, IPC through Mojo) per site. SexOS can put each site in its own domain
   on an organ key, with zero-copy shared regions between renderer, network and
   display. More sites isolated, less memory, faster switches.
5. **V8's sandbox on hardware keys.** V8's heap sandbox is a 1 TiB reservation that
   untrusted pointers cannot leave. Our arenas already give big reservations with
   page tables on demand; adding a hardware key around it turns V8's software
   sandbox into a hardware one.
6. **Faults as messages, not signals.** POSIX signals are the messiest part of every
   V8/Chromium port. SexOS can deliver a program's fault to a handler task as a
   message with the saved registers, and resume or end the run. Wasm bounds checks
   and V8's trap handler use this instead of `SIGSEGV`.
7. **No fork, no ambient authority.** A browser port here never needs `fork`,
   `/proc` or setuid sandboxes: capabilities already are the sandbox.

## 5. Plan

Each stage ends with a QEMU gate at 1 and 4 cores. Stages J1-J3 are worth showing on
bare metal next week.

| Stage | What | Kernel / SDK work | Showable result |
|---|---|---|---|
| **J1 (done here)** | Pages a program turns into code | `SYS_PROGRAM_CODE` 82 seal/open, key-instruction check at seal and at load, `sex_app::code::CodeBuf` | `run jit`: an expression compiler 3 to 9 times faster than its interpreter in QEMU; WRPKRU refused; W and E faults end only the run |
| J2 | Native Wasm | wasmtime runtime (no_std, `custom-virtual-memory`) in `sex-wasm`, modules precompiled by Cranelift at build time, loaded through `CodeBuf` | QuickJS and Wasm guests at native speed; pinball much faster on hardware |
| J3 | Threads | Program tasks sharing the run's arena and key; futex-style wait/wake; per-task stacks with guard pages; TLS via FS base | Multi-core canvas/WebGL raster, background JS compile |
| J4 | Faults a program handles | Fault message to a registered handler task with saved registers; resume, skip, or end | Wasm traps without bounds checks; V8 trap handler |
| J5 | C/C++ runtime | Grow `sex-libc` (or relibc) to what V8 needs: pthreads, `mmap`-like reserve/commit/protect over arenas, clocks, `abort`; libc++ and libc++abi built for `x86_64-sex-app` | A C++ "hello" with exceptions and threads |
| J6 | V8 jitless, then Sparkplug | `platform-sexos.cc` (memory via 81/82, threads via J3), GN toolchain for SexOS, Rust bindings in `sex-js` behind the same `__host` API | `web` runs pages on V8; QuickJS stays as fallback |
| J7 | Maglev and TurboFan | Background compile threads; out-of-domain compiler organ (section 4.3) | Real Chromium-class JS speed |
| J8 | Chromium content_shell | POSIX layer for `base/`, Ozone platform on Display/Input, network on sexnet, Skia on our surfaces; Helium-style patch set | Full web compatibility browser |

Rough sizes: J2 one week, J3 one to two weeks, J4 one week, J5 two to three weeks,
J6 three to four weeks, J7 two weeks, J8 months.

## 6. Risks

- **Byte check false positives.** A JIT may emit WRPKRU bytes by chance inside a
  constant. Today it gets the offset back and re-encodes (the `jit` demo does this).
  For V8 we either teach its assembler to avoid the three patterns, or move to an
  instruction-boundary check with control-flow rules. None of the 20 programs in the
  image contain the patterns (17 MB of code scanned), so the load-time check refuses
  nothing real.
- **Other ways to change keys.** XRSTOR and XRSTORS are refused. Programs cannot
  reach XSETBV or MSRs from ring 3. ERIM's analysis lists no other user-mode way.
- **One key per row.** A JIT engine with an out-of-domain compiler needs a second key
  per browser; organ keys and virtual keys (PROGRAM_SLOTS_DESIGN) cover this.
- **Build cost.** V8 and Chromium are heavy to build in cloud containers. Plan for a
  prebuilt toolchain image and cached object files.

## Done in this PR (J1)

- Kernel: `SYS_PROGRAM_CODE` (82), `kernel/src/app_resources.rs::code`. Only the
  current run's heap pages, at most 256 per call. SEAL checks the bytes (plus two
  bytes of any code neighbour on each side), clears W and NX, shoots down every core's
  TLB entry and updates the page manifest. OPEN sets W and NX again. Present bits are
  kept, so shared-key rows still take turns.
- Kernel loader: a program file whose executable segments hold WRPKRU, XRSTOR or
  XRSTORS is refused (`reason=key_instruction`). This closes a gap where any native
  program could unlock every key.
- ABI: new syscall 82 and `sex_pdx::program_code` (named here as an ABI change);
  `abi_version_hash` recomputed.
- SDK: `sex_app::code::CodeBuf` (page-aligned heap pages, `load`, `seal`, `open`,
  `entry`).
- Program: `apps/jit` (`run jit`).
- Gates: `scripts/program_jit_gate.py` (QEMU) and `scripts/program_code_host_gate.sh`
  (host tests for the byte check).
