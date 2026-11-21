# Crossdress: Windows programs in Drag cells (CW1, CX1, CW3 landed)

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

## CX1 (2026-11-18): a Windows CLAP plugin in a cell

Asked for: push Crossdress toward a first Windows audio plugin in a Drag cell. This step
proves the plugin half of CW8 (the yabridge role) before Wine's PE DLLs (CW3) exist.

- **Test plugin** `test/gain.c` → `test/gain.clap` (`test/build_gain.sh`): a real 64-bit
  Windows CLAP plugin DLL, "Crossdress Test Gain", built with clang + lld-link against the
  upstream CLAP headers (`vendor/clap-fixture/include`), no SDK, no C runtime. Stereo gain
  with one automatable parameter, its instance on the process heap, DllMain counting
  attaches, and every parameter change echoed through `out_events`. `_Static_assert`s pin
  every structure offset the host uses, so a header change breaks the plugin build, not
  the audio. Byte-compared with LLVM 18 like `hello.exe`; other LLVMs run the rebuild on
  KVM (`CROSSDRESS_CLAP`).
- **DLLs and host calls** (`process.rs`): `WinProcess::load_dll` maps a DLL as the cell's
  only module and resolves its exports; `call` runs any guest function with Windows x64
  arguments (stack arguments, 16-byte alignment, return through a call-return thunk);
  `double` arguments go through a 26-byte shim that loads XMM0-XMM3 from memory (CLAP's
  `activate(plugin, double, u32, u32)` needs XMM1); `dll_main` attach/detach.
  32 **host callback thunks** (`callback_va(n)`, `cell::CALLBACK | n`) are function
  pointers the guest can call; they reach `Host::callback`.
- **KERNEL32**: GetProcessHeap, HeapAlloc (bump heap mapped on demand, zeroed), HeapFree.
- **Bridge** (`clap.rs`): `ClapPlugin` lays out `clap_host_t`, `clap_process_t`, audio
  buffers and event lists in a bridge area of the cell; `open` (DllMain,
  `clap_entry.init`, factory, descriptor, `create_plugin`, `init`, `clap.params`),
  `activate`, sample-accurate `process` with CLAP_EVENT_PARAM_VALUE in and out,
  `param_value`, `close`. Guest pointers are read with user rights only.
- **Program**: `run crossdress plugin` runs the built-in plugin and checks its audio bit for
  bit; `run crossdress FILE.clap` opens another Windows CLAP plugin and processes a test
  signal (serial `[crossdress.clap.*]`).

Gates: `tests/kvm.rs` `a_windows_clap_plugin_processes_audio_in_a_kvm_cell` (CI host job,
real SVM/VT-x): three blocks, gain changes at frames 100 and 300, every output sample
bit-identical to the same arithmetic on the host, both changes reported back, callbacks
counted. `scripts/drag_gate.py` runs `run crossdress plugin` (TCG: loaded then refused;
KVM: `exact=1 reported=2`).

What real closed plugins still need, in order: a C runtime and the rest of KERNEL32/NTDLL
(CW3: Wine's PE DLLs, `__wine_unix_call` to the host), threads as vCPUs (CW4), VST3's COM
interfaces over the same call/callback seams, plugin GUIs (CW6), and Moan's plugin host
(moanplug) opening Windows `.clap`/`.vst3` files through `ClapPlugin` in its sandboxed
helper.

## CW3a (2026-11-19): Wine's ntdll.dll in a cell

Andreas approved CW3 (Wine's Windows libraries) on 2026-11-18.

- **Pinned Wine**: 9.0 as Ubuntu 24.04 ships it (`scripts/fetch_wine_pe.sh`, SHA-256
  checked; ledger table 5). Only the PE halves are used.
- **Seams** (`src/nt.rs`): Wine 9's `Nt*` stubs test KUSER_SHARED_DATA+0x308 and call
  `[0x7FFE1000]`; Crossdress maps KUSER with the flag set and the pointer at the cell's
  syscall dispatcher thunk (`mov r11d, eax; mov eax, SYSCALL; out 0xE0, eax; ret`).
  `__wine_unix_call_dispatcher`, `__wine_syscall_dispatcher` and `__wine_unixlib_handle`
  are filled the way the Unix half would. Syscall numbers are read from the stubs, so the
  host matches Wine builds by name. Debug options live at PEB+0x2000 (empty: all off).
- **NT system calls served**: NtAllocateVirtualMemory, NtFreeVirtualMemory (no unmap yet),
  NtQuerySystemTime, NtQueryPerformanceCounter, NtClose, NtYieldExecution,
  NtTerminateProcess. Any other stops cleanly as `ntdll.syscall!NtName`; a Unix call stops
  as `unixlib#handle!code N`.
- **Native test mode** (`tests/native.rs`): cell user pages mapped at their own addresses
  in the Linux test process, entry/exit through SIGUSR1/SIGSEGV with the guest's `out`,
  GS = TEB. Lets Wine be iterated on without /dev/kvm; the CLAP plugin and ntdll run there.

Gate (`tests/common`, run natively and in `tests/kvm.rs`): Wine's RtlComputeCrc32,
ntdll's own sprintf, NtQuerySystemTime through Wine's stub and the dispatcher, then Wine's
real heap (RtlCreateHeap, RtlAllocateHeap zeroed, RtlSizeHeap, RtlFreeHeap) on top of the
host's NtAllocateVirtualMemory.

## CW3b (2026-11-19): a Windows program on Wine's kernel32

`src/wine.rs` starts a program the way Wine's Unix half does: the host maps ntdll.dll and
the program, builds the PEB, RTL_USER_PROCESS_PARAMETERS (normalized, with environment),
the TEB fields Wine's `init_teb` fills (activation context stack, static Unicode string,
real client id) and a virtual `C:\windows\system32` holding Wine's DLLs and NLS tables.
Then it calls ntdll's own `LdrInitializeThunk(context)`. Wine's loader does the rest in
the cell: it opens and maps kernel32.dll and kernelbase.dll through NtOpenFile,
NtCreateSection and NtMapViewOfSection (with relocation), fixes the program's imports,
runs DllMain, and reaches the program through NtContinue, RtlUserThreadStart and
BaseThreadInitThunk. The test program (`test/wine_hello.c`, built with clang + lld-link,
no SDK) calls HeapAlloc, GetModuleHandleA, GetProcAddress, lstrlenA, GetCommandLineA,
GetCurrentProcessId and WriteFile on Wine's real kernel32, prints
`crossdress: wine checks passed` and exits 0 through NtTerminateProcess. 87 system calls,
no host-side kernel32.

- **Served on top of CW3a**: NtProtectVirtualMemory (splits cell mappings, rewrites
  PTEs), NtQueryVirtualMemory, NtContinue, NtOpenFile/NtCreateFile, NtQuery(Full)
  AttributesFile, NtCreateSection/NtQuerySection (SEC_IMAGE), NtMap/UnmapViewOfSection,
  NtFsControlFile (FSCTL_GET_OBJECT_ID), NtQueryInformationProcess/Thread/Token,
  NtQuerySystemInformation, NtQueryVolumeInformationFile, NtWriteFile (console),
  NtInitializeNlsFiles, NtGetNlsSectionPtr, NtQueryDefault{Locale,UILanguage}; registry
  opens answer "not found" (no registry yet).
- **Gate**: `a_program_runs_on_wines_kernel32_{natively,in_a_kvm_cell}`; the host gate
  rebuilds wine_hello.exe and compares it byte for byte (LLVM 18), or runs the rebuild
  with `CROSSDRESS_WINE_EXE` on another LLVM. `CROSSDRESS_NT_TRACE=1` prints every call.

## CW4 (2026-11-19): Wine's C runtime, the MSVC way

- **CW4a**: `test/wine_crt.exe` links `ucrtbase.dll` directly (malloc, calloc, strtol,
  qsort with a callback into the program, sqrt, sprintf, puts, exit) and passes on Wine's
  ucrtbase. The host's std handles are pipes (FILE_TYPE_PIPE), so the CRT buffers lines;
  NtDeviceIoControlFile (condrv) is refused, so GetConsoleMode fails and output goes
  through NtWriteFile. NtTerminateProcess(0, ...) ends only the *other* threads (none),
  so DllMain(DETACH) runs and flushes stdio before NtTerminateProcess(-1).
- **CW4b**: the host maps Wine's `apisetschema.dll` read-only and points PEB.ApiSetMap at
  its `.apiset` section, as Wine's Unix loader does. `test/wine_msvc.exe` imports
  `api-ms-win-crt-{heap,string,stdio,math,runtime}-l1-1-0.dll` and `VCRUNTIME140.dll`
  exactly as MSVC builds do, and Wine's ntdll resolves them to ucrtbase and vcruntime140.

## CW5 (2026-11-19): CLAP plugins loaded by Wine's own loader

- `wine::start` runs `LdrInitializeThunk` with a CONTEXT that continues at the host
  call's return thunk instead of RtlUserThreadStart: Wine initializes the process (kernel32,
  kernelbase, their DllMain) and hands it back without running the program.
- `wine::load_library` calls Wine's `LdrLoadDll` for a plugin in the virtual file system;
  imports (through API sets), relocation and DllMain are Wine's.
- `ClapPlugin::in_wine` drives such a plugin with the CX1 bridge (DllMain already ran).
- `test/gain_msvc.clap` is the test plugin built with `CROSSDRESS_MSVC_CRT`: its memory
  comes from `calloc`/`free` through `api-ms-win-crt-heap` and `memcpy` from
  `VCRUNTIME140.dll`. Both plugins process the CX1 audio bit-exact in a Wine process.
- Fixed on the way: NtMapViewOfSection reused a view whenever *any* image sat at the
  preferred base (ucrtbase and the plugin both prefer 0x180000000); views are now keyed by
  file and a clash is relocated. Unix call 1 (`unwind_builtin_dll`) answers "nothing" so
  Wine's unwinder treats thunk frames as leaves; an unhandled exception stops the cell
  with its code, and EXCEPTION_WINE_STUB with the DLL and function it names.

Next: msvcp140 (C++ plugins), a small registry, VST3 (COM), and Moan opening Windows
plugins.
