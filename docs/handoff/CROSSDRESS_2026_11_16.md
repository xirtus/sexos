# Crossdress: Windows programs in Drag cells (CW1 landed)

Status: CW1 (the cell proof) is in this PR. Andreas started the workstream on 2026-11-16
("start it now, build proof of crossdress, SexOS's own equivalent to Wine"). The design
and the comparison with Wine, Bottles, yabridge and VMs are in
`docs/handoff/WINDOWS_COMPAT_STUDY_2026_11_16.md`.

## What CW1 proves

An unmodified 64-bit Windows `.exe` runs in a **Drag cell**: a SYS_VM (syscall 110)
guest with its own page tables and no guest operating system. Everything a Windows
kernel would do is done by the host program on VM exits. No kernel change.

The test program is a real Windows console program (`crates/sex-crossdress/test/hello.c`,
built with clang and lld-link, no Windows SDK and no C runtime; deterministic build, the
host gate checks the checked-in `.exe` against its source). It uses only documented
KERNEL32 functions, so it runs on Windows too. It checks:

1. console output through `GetStdHandle` + `WriteFile`;
2. the TEB through `gs:[0x30]`, the PEB through `gs:[0x60]`, and that
   `PEB->ImageBaseAddress` is the image;
3. that it was loaded at its preferred base 0x140000000 and its absolute pointers are
   right;
4. `GetCurrentThreadId` agrees with the TEB;
5. `VirtualAlloc` anywhere, at a fixed low address (0x10000000, impossible in SexOS's
   shared address space), and reserve-only memory that faults;
6. a vectored exception handler (`AddVectoredExceptionHandler`) that catches a null
   read, a write to `.rdata`, a read of reserved memory, `int3` and a divide by zero,
   each with the Windows status code and access-violation details, and resumes by
   editing the CONTEXT.

## How it is built

| Piece | Where | What |
|---|---|---|
| PE loader | `crates/sex-crossdress/src/pe.rs` | PE32+ headers, sections, base relocations (DIR64/HIGHLOW), imports by name and ordinal, exports. Hostile headers and every truncation are refused without panics. |
| Cell | `src/cell.rs` | The cell's memory is one block of the program's heap lent at guest-physical 0. 4-level page tables written by the host (user/supervisor, read-only, NX). GDT with Windows's selectors (0x10/0x18 kernel, 0x33/0x2B user, 0x23 for WoW64 later, TSS 0x40). TSS with IST1 and an I/O bitmap that opens only port 0xE0. 32 IDT gates to 32-byte ring-0 stubs that push CR2 and RAX and report the vector. |
| Process | `src/process.rs` | Image at its preferred base with per-section rights (W^X holds in the cell), PEB, TEB (GS base), stack, one 8-byte import thunk per import (`mov eax, CALL\|n; out 0xE0, eax; ret`), the exit loop, and vectored exception dispatch: EXCEPTION_RECORD + CONTEXT + EXCEPTION_POINTERS on the thread's stack, the handler called in ring 3, its verdict read through a return thunk. |
| KERNEL32 subset | `src/win32.rs` | GetStdHandle, WriteFile, ExitProcess, Add/RemoveVectoredExceptionHandler, VirtualAlloc, GetCurrentThreadId/ProcessId, GetTickCount64, Get/SetLastError, Sleep, QueryPerformanceCounter/Frequency. Unknown imports load and stop the program cleanly when called (`[crossdress.unimplemented] dll!name`). |
| Program | `apps/crossdress` | `run crossdress` runs the built-in test program; `run crossdress FILE.exe` runs one from its folder. Glue to SYS_VM like `apps/drag` (FXSAVE swap around each run). |

Hypercalls are `out 0xE0, eax` rather than `vmcall`. SexOS's SYS_VM gives an I/O exit
with the registers, and Linux KVM hands I/O exits to user space too (it handles `vmcall`
itself), so the same cell runs under KVM in CI. EAX says what the call is.

## Why this keeps SexOS's design

- The Windows code never runs in SexOS's single address space, so the loader's
  WRPKRU/XRSTOR byte check does not apply to it and is not weakened. The guest has its
  own PKRU, and D1 restores the host's on every exit.
- The program holds the Machine capability and its own heap; the guest sees only the
  lent block.
- No syscall was added and no kernel line changed.

## Tests

- `cargo test -p sex-crossdress` (in `scripts/drag_host_gate.sh`, CI host job): 17 unit
  tests. They cover the PE loader on the real test `.exe`, relocation, hostile headers,
  page tables checked with Drag's own guest page walker, descriptors, gates, the I/O
  bitmap, the stub bytes, user copies honouring page rights, KERNEL32 calls end to end
  against a scripted vCPU, a page fault dispatched to a handler and resumed from the
  edited CONTEXT, unhandled and ring-0 faults, and VirtualAlloc reserve/commit/refuse.
- The `.exe` rebuild is byte-compared only with the LLVM major it was built with
  (`test/LLVM_MAJOR`, 18). With another LLVM (CI has 21) the gate runs that rebuild in the
  KVM cell instead, so both builds must pass.
- `tests/kvm.rs`: the test `.exe` in a real VT-x/SVM cell on Linux KVM, through the same
  `Process`. It must print `crossdress: all checks passed`, exit 0 and handle five
  exceptions. It prints SKIP where `/dev/kvm` is not usable (this cloud container has
  none). CI makes `/dev/kvm` usable when the runner has it.
- `scripts/drag_gate.py` (devices job, QEMU TCG): `run crossdress` must load the PE and
  build the cell inside SexOS (`[crossdress.loaded] base=0x140000000 ... imports=7`), then
  be refused cleanly (no VMX under TCG). With `--accel kvm` (nested VMX) or on hardware,
  the program must run and pass its checks.

## On the x17 R1

Boot SexOS, open Spindle and type `run crossdress`. The window shows the program's lines
and ends with `crossdress: all checks passed`. The serial log has
`[crossdress.stop] reason=exited code=0x0 ... handled=5`.

## Known limits of CW1, and next

- One thread, no SEH unwinding (`__try`/C++ exceptions need `.pdata` unwinding, which
  Wine's ntdll does in the guest from CW3), no TLS callbacks, no DLL loading.
- Guest `syscall` instructions are not supported: D1 does not switch STAR/LSTAR/SFMASK
  for guests. Wine's dispatcher does not need them; raw-syscall programs (some DRM) need
  a small VMCS MSR-load addition later.
- Changing rights on pages the guest has already used (VirtualProtect) needs a guest TLB
  flush; CW1 only adds new mappings.
- Next: CW2, the in-SAS test mode for CI without KVM. CW3 builds Wine's PE DLLs (llvm-mingw)
  and routes `__wine_unix_call` to the host, replacing `win32.rs`.
