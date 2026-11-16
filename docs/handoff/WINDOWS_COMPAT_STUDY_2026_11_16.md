# Windows programs and plugins on SexOS: options, costs, recommendation

Status: study for Andreas's decision (2026-11-16). Report only, no code.
Asked in the Splicy thread: before any Windows work, find out whether there is a better
way to run Windows plugins (VST2/VST3/CLAP/OFX `.dll`) and apps than a Wine bridge,
Bottles, a yabridge-style host or a VM, and whether porting Wine can work well without
losing what the microkernel gives us. Splicy and Moan proceed with open plugins first
(OFX, Frei0r, CLAP, VST3 built for SexOS); this study does not block them.

Inputs: `docs/DESIGN_GUARDRAILS.md`, `docs/BRIDGE_LEDGER.md`,
`docs/handoff/DRAG_VIRTUAL_MACHINES_DESIGN_2026_11_14.md` (SYS_VM, D1 landed),
`docs/handoff/JIT_AND_BROWSER_ENGINE_PLAN_2026_11_09.md` (SYS_PROGRAM_CODE and the
WRPKRU byte check), `docs/handoff/MOAN_2026_11_14.md`, `docs/handoff/SPLICY_PLAN_2026_11_14.md`.

## 1. Short answer

Yes, Windows compatibility is realistic, and it should be its own workstream. But a
straight Wine port into the single address space is the wrong shape, for one hard reason
(section 3.1): foreign Windows machine code cannot be trusted next to PKU, and we cannot
rebuild it to pass the WRPKRU byte check.

The recommended direction is a hybrid we can call a **Drag cell**:

- Each Windows program (or plugin host) runs its unmodified PE code, plus Wine's own PE
  DLLs (ntdll.dll, kernel32.dll, user32.dll, ...), inside a tiny VT-x/SVM guest with **no
  guest operating system**: one vCPU per Windows thread, its own page tables and its own
  PKRU, and only the memory we lend it.
- Wine's "Unix side" (the part that talks to the host OS) is ported to SexOS and runs as
  ordinary SexOS code in the host run, and `wineserver` becomes a SexOS server.
- The boundary between them is Wine's own `__wine_unix_call` / syscall dispatcher, which
  becomes a `VMCALL`. Wine already split itself at exactly this line (the PE/Unix split,
  done over Wine 6 to 9), so we port the half that is OS-specific and reuse the rest as
  binaries.

This keeps the microkernel's properties (a Windows program holds only the caps it was
given, can never touch a host protection key, and adds almost nothing to the kernel),
runs plugin DSP at native speed, and also fixes the things a single address space makes
hard for Windows: fixed image bases, 32-bit programs, SEH from hardware faults,
`SuspendThread`/`GetThreadContext`, and programs that issue raw `syscall` instructions.

A full Windows guest in Drag (D6) stays the catch-all for what Wine cannot do (iLok/PACE,
kernel drivers, anti-cheat). A self-written Win32 reimplementation is not worth it: Wine
already is that, and we would reuse its PE half unchanged.

Working name proposal: **Crossdress** (Windows programs dressed as SexOS ones, built on
Drag). Alternatives: Merlot, Negligee. Corset is taken (disk UI).

## 2. What a Windows binary expects, and how SexOS differs

| Windows/Wine expectation | Linux gives Wine | SexOS today | Gap size |
|---|---|---|---|
| Its own address space; EXE at its preferred base (0x140000000), DLLs relocatable | per-process page tables, `mmap(MAP_FIXED)` | one global address space; a run gets heap pages wherever they are free | DLLs: fine (always relocatable). EXEs built `/FIXED` or with stripped relocs: impossible in-SAS |
| 32-bit programs and plugins in the low 4 GiB (WoW64) | Wine's preloader reserves the low 4 GiB per process | the low 4 GiB is shared by everyone | impossible in-SAS without binary translation |
| TEB at `gs:[0x30]`, set per thread | `arch_prctl(ARCH_SET_GS)` | per-thread `fs_base` exists (J5a); user `gs` base is not switched, CR4.FSGSBASE off | small kernel mechanism |
| Hardware faults become SEH exceptions (`__try/__except`, access violations used on purpose by .NET, Java, some DRM, guard pages) | `SIGSEGV`/`SIGILL` handlers, then `KiUserExceptionDispatcher` | J4: a user fault ends the run | new kernel upcall mechanism |
| `SuspendThread`, `Get/SetThreadContext` (.NET GC, debuggers, profilers, DRM) | signals (`SIGUSR1`) plus `ptrace` in wineserver | no op for that | new kernel mechanism |
| NT sync objects (events, mutexes, semaphores, `WaitForMultipleObjects`) | wineserver round trips, esync/fsync, `ntsync` (Linux 6.14) | wait/wake (J3) and PDX IPC | library + server, no kernel change |
| Files, drive letters, registry | Unix filesystem, `Z:` = `/` | a run holds only the files it was granted | good fit: `C:` = a prefix folder grant, no `Z:` |
| Process creation, handles across processes | `fork`/`posix_spawn`, fd passing over Unix sockets | Shell launch request, grants, PDX | server design, no kernel change |
| Code bytes are just code | nothing checks them | **every executable page is scanned for WRPKRU/XRSTOR/XRSTORS at any offset** (J1) | **blocking**, section 3.1 |

## 3. The options

### 3.1 Option A: port Wine into the single address space (Wine as a userspace library)

What it is: build Wine's Unix side against `sex-libc`/`sex-std`, wineserver as a SexOS
server, Wine's PE DLLs and the Windows program loaded straight into a SexOS run, the way
Wine runs on Linux. This is what "Wine as a compat library" means, and it matches
guardrail section 2 on paper.

Why it does not hold up:

1. **The byte check refuses foreign code.** SexOS's isolation is PKU. Any code that can
   execute `WRPKRU` (or `XRSTOR` with PKRU in the mask) can open every key and read the
   whole system, so the loader refuses any image with those bytes at any offset, and the
   fix for our own code is to rebuild it (`scripts/key_bytes.py`, PR #53). We cannot
   rebuild a Windows plugin. Rough odds if the bytes were random: in 10 MiB of code the
   3-byte `0F 01 EF` (WRPKRU) appears about 0.6 times and the memory forms of
   `0F AE /5` (XRSTOR) about 15 times. FFmpeg alone tripped the check. So most large
   Windows binaries would be refused, and relaxing the check to instruction boundaries
   is unsound without control-flow integrity (the J1 decision). Every JIT inside a
   Windows program (.NET, Java, LuaJIT inside synths) hits the same wall at run time.
2. **No 32-bit programs and no fixed-base EXEs**, because the low 4 GiB and fixed bases
   are shared.
3. **Kernel growth for Linux-like features**: thread suspend/context, a fault upcall,
   a user GS base. Each is small (about 100 + 150 + 30 lines), but the first two are
   signal-shaped and need care to stay within guardrail section 2.

Verdict: fine for code we build ourselves (it is how `sex-libc` already works), not for
the foreign binaries this whole exercise is about. Its one good use is as a **test mode**
(section 5): Wine's own PE DLLs and our test programs, rebuilt to pass the byte check,
can run in-SAS in CI where there is no VT-x.

### 3.2 Option B (recommended): Wine split across a Drag cell

What it is: SYS_VM (Drag D1, landed) already gives a ring-3 server a VT-x guest whose
memory is pages from its own heap. A Drag cell uses that **without a guest kernel**:

```
 host run (SexOS, ring 3)                       Drag cell (VMX non-root)
 +-------------------------------+              +---------------------------------+
 | cell host: one worker per     |  VCPU_RUN    | guest ring 3: plugin.dll / app  |
 | Windows thread                | -----------> |   + Wine PE DLLs (unchanged)    |
 | Wine Unix side ported to      |              | guest ring 0: cell stub (~1.5k  |
 | SexOS: ntdll unix, win32u     |  VMCALL exit |   lines Rust): IDT, SEH upcall, |
 | unix, winesex.drv (Silk),     | <----------- |   syscall entry -> VMCALL       |
 | winebell.drv (Bell), GL/VK    |              +---------------------------------+
 | thunks                        |   guest pages = host heap pages, mapped at the
 +--------------+----------------+   SAME virtual addresses (identity cell), so
                | PDX                Unix-side code reads guest pointers directly
        wineserver as a SexOS server (handles, sync, windows, processes, registry)
```

How each gap from section 2 closes:

| Gap | In a cell |
|---|---|
| WRPKRU byte check | not needed for guest code: the guest runs with its own CR4/PKRU in the VMCS and D1 already restores host PKRU on every exit. The guest can only touch pages lent to its EPT. |
| Fixed bases, 32-bit, WoW64 | the guest has its own page tables, so it can have its own low 4 GiB and any base. 64-bit guest addresses default to the identity mapping; guest-private ranges (low 4 GiB, fixed bases) are the only addresses the Unix side must translate, which Wine's WoW64 thunks already do. |
| TEB in `gs` | the guest's own GS base, set by the cell stub. No kernel change. |
| SEH from hardware faults | the guest IDT catches the fault inside the guest and builds the `KiUserExceptionDispatcher` frame there. No host kernel change and no exit. |
| Suspend / thread context | the host already owns every vCPU's registers (`VCPU_GET/SET_REGS`). No kernel change. |
| Raw `syscall` from DRM or game code | lands in the cell stub, becomes a VMCALL. Works like Wine's own dispatcher. |
| Sync objects | wineserver + ntsync-style objects on wait/wake: a blocking wait is a VMCALL that parks the host worker. |

Kernel cost: nothing new for a first version. It uses SYS_VM as landed. Performance needs
Drag D4 (keep VMX on and the VMCS loaded between runs, VPID, multiple vCPUs per VM),
which Drag wants anyway, and AMD needs D5 (SVM).

Performance:
- Pure computation (plugin DSP, codecs inside an app) runs as native code. EPT adds a
  two-level page walk on TLB misses, typically 0 to 5 percent.
- Each host call costs one VM exit and entry: about 1 to 2 µs with D4, more with D1's
  per-run VMXON/VMXOFF. An audio plugin's `process()` needs one resume per buffer
  (1.3 ms at 64 samples, 48 kHz), so that is under 0.2 percent. GUI calls are batched
  by Wine already. yabridge on Linux pays a socket round trip per buffer, which costs
  more than this.
- Audio and video buffers are zero-copy: they are host heap pages the guest sees at the
  same address.

Security: better than Wine on Linux. A Windows program is a confined run with its own
grants (its prefix folder, files the user opened, a window, a Bell stream, `+net` only
if granted); a hostile plugin cannot reach the host's keys or memory. It also isolates
plugins from the DAW: a crashing plugin ends its cell, as Moan's sandbox already
requires.

Guardrail fit: the kernel keeps mechanism only (SYS_VM); the compat layer is userspace
(the cell host library and a server); no Linux-shaped syscall is added. Wine is
LGPL-2.1-or-later, which is compatible with our GPL-3 code; the Unix-side port is a
derived work under LGPL. It is not a driver bridge, so it gets no Bridge Ledger row, but
the study suggests a small "foreign code" table in the ledger listing Wine and its
version, like table 4.

Costs and risks:
- **Needs VT-x or SVM.** Every target PC (the x17 R1 included) has it. QEMU TCG in CI
  does not; the first slice must check whether a KVM runner with nested VMX can run
  SexOS's cells, and otherwise the gates run in-SAS test mode (section 5) plus receipts
  from real hardware.
- Wine's Unix side is written for Linux/macOS: `dlls/ntdll/unix` (virtual memory,
  threads, signals, files, sync, loader), `server/` (wineserver, about 45k lines, poll and
  fd-passing based), the display drivers (`winex11.drv` is about 30k lines; ours,
  `winesex.drv` on Silk, would be new code of maybe 8 to 12k lines), audio
  (`winepulse.drv`-sized, about 3k lines for Bell), and `dlopen` of `.so` Unix libraries
  (we link them statically into the cell host with a name table instead).
- Graphics: Wine's Direct2D and Direct3D go through wined3d, which needs desktop OpenGL
  or Vulkan. SexOS has `sex-gl` (a GLSL ES interpreter for WebGL) and no Vulkan yet. Many
  plugin UIs draw with GDI or software and work without it; JUCE plugins that use
  Direct2D or OpenGL, and every 3D app, wait for a GL/Vulkan path (Mesa llvmpipe/lavapipe
  in software, then the Intel/NVIDIA work). This is the biggest external dependency.
- Each running Windows program costs a cell (VMCS and EPT per vCPU, a few hundred KiB),
  far less than a guest OS.

Precedents for running code in a VM with no guest kernel: gVisor's KVM platform, Stanford
Dune (2012), Microsoft Hyperlight (2024).

### 3.3 Option C: Wine inside a Linux guest in Drag

Run a small Linux guest (D2/D3) with Wine and a yabridge-like server; Moan and Splicy
talk to it over a shared-memory ring.

- Pro: least new porting. Wine runs as on any Linux; yabridge's model already works.
- Con: a whole Linux guest (about 0.5 to 1 GiB RAM, seconds to boot) per session, audio
  and video cross a virtio boundary (one extra buffer of latency), windows come through
  a guest framebuffer or need a guest agent for per-window display, files cross through
  virtio-fs. Linux userspace becomes a permanent tenant for a daily feature, which is the
  drift the guardrails exist to stop.
- Effort: after Drag D3/D4, about 8 to 12 slices for a plugin bridge that works.

Verdict: a reasonable stopgap if plugins are needed before the cell work lands, not the
direction.

### 3.4 Option D: a full Windows guest in Drag (D6)

- Pro: runs everything, including iLok/PACE (USB passthrough or iLok Cloud), vendor
  installers with kernel drivers, and CUDA plugins if a GPU is passed through.
- Con: a Windows licence, 4+ GiB RAM, a boot, a whole Windows desktop in a window, a
  bridge service we write for the Windows side, and Drag D6 (OVMF, virtio-win drivers,
  display) first. Some DRM and anti-cheat refuse to run in VMs.

Verdict: keep as the catch-all for the minority Wine cannot run. It comes with D6.

### 3.5 Option E: our own PE loader and a selective Win32 reimplementation

The surface a "plugin-only" Win32 needs is still large. JUCE, iPlug2 and similar
frameworks use kernel32, user32, gdi32, d2d1, dwrite, ole32/COM, shell32, comdlg32,
winhttp/wininet and crypt32 for licensing, plus the C runtimes (ucrtbase, msvcp140).
Wine has spent about 30 years on exactly this, and since its PE/Unix split those DLLs are
plain PE binaries we can load unchanged. Writing our own (in Rust) would duplicate them
for years and still be less compatible.

Verdict: no. The "own" part worth writing is only the cell (loader, stub, host) and the
SexOS Unix side of Wine, which Option B already is.

### 3.6 The plugin host itself (yabridge-style), whichever runtime

This is not an alternative but a layer every option needs: Moan and Splicy should never
load a foreign `.dll` into their own run. A plugin is hosted by a bridge program that
speaks VST2/VST3/CLAP (audio) or OFX (video) to the app over shared memory, the same
model as Moan's restricted `moanplug` helper (PR #347). In Option B the bridge's Windows
half runs in the cell and its app half is our CLAP/VST3 host code. One host per plugin
(isolation) or one per vendor (shared state) is a setting.

### 3.7 Baseline: Wine + yabridge on Linux (what we measure ourselves against)

Andreas asked (2026-11-16) for Wine + yabridge to be the baseline, since it is how Linux
musicians run Windows VST2/VST3 plugins today. How it works:

- `yabridgectl sync` puts a small native Linux "chainloader" library (a `.so`, `.vst3`
  or `.clap`) next to each Windows plugin. The DAW loads it like any Linux plugin.
- The chainloader loads yabridge's plugin bridge, which starts `yabridge-host.exe`, a
  Winelib program, in the plugin's Wine prefix. The host loads the Windows `.dll`.
- The two halves exchange VST2/VST3/CLAP calls as serialized messages over Unix
  sockets. Since yabridge 4, audio buffers go through shared memory, so the per-buffer
  cost is one wakeup each way.
- "Plugin groups" put several plugins in one host process, so plugins from one vendor
  can share state and memory.
- The plugin's editor is a Wine window reparented into the DAW's X11 window (X11 only,
  no native Wayland).
- Formats: VST2, VST3 and CLAP (CLAP since yabridge 5.1). Licence: GPL-3.0-or-later,
  the same as ours.
- Known limits: results depend on the Wine version (some plugins need wine-staging or
  an older Wine), iLok/PACE plugins do not run, DPI and focus quirks, and installers
  and licence managers (Native Access, Waves Central) are hit and miss.

**Video (OFX) has no baseline.** No yabridge exists for OFX. Linux Resolve, Natron and
Nuke load only native Linux OFX builds, and Windows OFX packs (Boris FX, Red Giant,
NewBlue) mostly need CUDA, OpenCL, D3D or Metal anyway. A Windows OFX bridge would be
new work, and is only worth it for CPU-rendered effects.

**What we take from yabridge.** Its layout is the right one, and Option B keeps it:

| yabridge on Linux | SexOS (Option B) |
|---|---|
| chainloader `.so` in the DAW | the plugin host side in Moan's `moanplug` sandbox and in Splicy |
| `yabridge-host.exe` in Wine | the same host program, built as a Windows binary, in a Drag cell |
| Unix sockets + shared-memory audio | shared-memory rings (lent pages, zero-copy) + wait/wake |
| plugin groups | several plugins in one cell, a per-vendor setting |
| X11 reparenting for the editor | `winesex.drv` gives the plugin window as a child of the Moan/Splicy Silk surface |
| Wine prefix | a per-prefix folder grant (`C:`), no access to the rest of the system |

Because yabridge is GPL-3, we can port its code directly: its message types for
VST2/VST3/CLAP, its threading model (it mirrors the plugin's audio, GUI and host
callback threads) and its many per-plugin workarounds. Only the transport (sockets to
shared-memory rings) and the editor embedding change. CW8 becomes "port yabridge".

**Benchmark.** The baseline is measured on the same machine: the x17 R1 dual-boots
Linux, so the same plugins can run in Linux (Wine + yabridge in a DAW) and in SexOS
(Moan). We compare three things: which plugins load and open their editor (a shared
compatibility list), DSP load at 64 and 128 samples, and round-trip latency. The goal
is to match yabridge's compatibility list and beat its per-buffer overhead.

## 4. Comparison

| | A: Wine in-SAS | **B: Wine in Drag cell** | C: Linux guest + Wine | D: Windows guest | E: own Win32 |
|---|---|---|---|---|---|
| Runs unmodified 64-bit plugins | most refused by byte check | yes | yes | yes | partially |
| 32-bit plugins and apps | no | yes (WoW64 in the cell) | yes | yes | no |
| DRM (iLok/PACE), anti-cheat | no | some (Wine's limits) | some | most | no |
| DSP speed | native | native (0 to 5 percent EPT) | native, plus virtio latency | native, plus virtio latency | native |
| Integration (windows, files, audio) | full | full | poor to fair | poor | full |
| Isolation | weak (PKU unsafe) | strong | strong | strong | depends |
| New kernel code | ~280 lines, signal-shaped | none (uses SYS_VM; D4/D5 help) | none | none | ~280 lines |
| Hardware needs | none | VT-x or SVM | VT-x or SVM | VT-x or SVM | none |
| Guardrail fit | poor (security) | good | weak (Linux tenant) | fair | good |
| Effort to first working plugin | ~20 slices, then blocked | ~25 to 35 slices | ~8 to 12 after D3/D4 | after D6, ~8 more | years |

A "slice" is a PR-sized step with a gate, as in the other plans.

## 5. How Option B becomes its own workstream

Proposed as its own project thread ("Crossdress"), after Andreas decides. It depends on
Drag D4 (performance) and on a GL/Vulkan path for D2D/D3D UIs; the first slices do not.

| Slice | Delivers | Gate |
|---|---|---|
| CW0 | Decision, name, ledger "foreign code" table, Wine version pinned (a current stable release) | review |
| CW1 | Cell host and cell stub: load a tiny mingw-built 64-bit PE exe (no Wine) into a cell, identity mapping, GS/TEB, one hypercall (`write`), SEH from an access violation | VT-x hardware receipt; check nested VMX on a KVM CI runner |
| CW2 | In-SAS test mode: the same host code runs self-built, byte-checked PE code without a cell, so CI on TCG can gate everything above the transport | CI gate |
| CW3 | Wine build for SexOS: PE half with llvm-mingw as is; Unix half statically linked into the cell host against `sex-libc`; `__wine_unix_call` and the syscall dispatcher go to VMCALL | `wine cmd /c echo` |
| CW4 | ntdll Unix side: virtual memory over lent pages, threads as vCPUs, files on grants (`C:` = prefix folder), ntsync-style objects on wait/wake | Wine's `ntdll` and `kernel32` conformance tests (winetest), pass rate tracked |
| CW5 | wineserver as a SexOS server over PDX: handles, processes (CreateProcess = Shell launch), registry, message queues | `reg`, `start`, two processes sharing an event |
| CW6 | `winesex.drv`: Silk windows, input, clipboard, cursor, DPI; GDI | `notepad.exe`, `winemine` in Silk windows |
| CW7 | `winebell.drv` (mmdevapi/WASAPI on Bell) | a Windows player plays sound |
| CW8 | Port yabridge (section 3.7): its VST2/VST3/CLAP host in a cell, shared-memory transport instead of sockets, plugin groups, editor embedded in Moan | free closed plugins (TDR Nova, Valhalla Supermassive, Voxengo SPAN) process audio and open their UI; compared with Wine + yabridge on the x17 R1's Linux |
| CW8b | OFX bridge for Splicy, built the yabridge way (new: no Linux baseline exists), CPU-rendered effects only | a free Windows OFX effect renders a frame identical to Windows |
| CW9 | WoW64 in cells: 32-bit plugins and apps | a 32-bit VST2 processes audio |
| CW10 | Graphics: wined3d on a SexOS GL (Mesa llvmpipe first), then DXVK when Vulkan exists | a D2D plugin UI; a D3D11 sample |
| CW11 | Apps: wine-mono (.NET), MSI installers, fonts, file associations in Files | an installer runs end to end |

Rough size: CW1 to CW8 (plugins that draw with GDI or software) about 25 to 35 slices;
porting yabridge rather than writing a bridge from scratch is what keeps CW8 small.
CW9 to CW11 another 20 or more, and the D3D part is gated on the GPU work. Wine
compatibility after that is open-ended, as it is on Linux, but most of it is upstream
Wine improving and us updating.

What it does not cover:
- GPU plugins that need CUDA or Metal (many commercial OFX effects): only Option D with
  GPU passthrough.
- iLok/PACE and other kernel-driver DRM: Option D.
- macOS `.component`/`.vst3`/`.plugin` bundles: Mach-O plus Cocoa; Darling is far from
  usable and Apple's frameworks are not reimplementable at reasonable cost. Out of scope;
  the realistic route for Mac-only plugins is that their vendors ship Windows or CLAP
  builds.
- Linux-only plugin `.so` files (glibc, X11): the same cell idea could host them later
  with a Linux-ABI stub, but open plugins rebuilt for SexOS are the better path.

## 6. Recommendation

1. Keep "open plugins first" in Splicy and Moan as decided: it is the cheapest, fastest
   and fully native path, and the bridge layer (section 3.6) it builds is reused for
   Windows plugins later.
2. Do not port Wine into the single address space (Option A). It cannot run untrusted
   Windows code safely with PKU.
3. Make Windows compatibility its own workstream on the Drag-cell design (Option B),
   starting with CW1 and CW2 to prove the cell on real VT-x hardware, since that proof
   decides everything after it. Drag D4 (keep VMX on, multiple vCPUs) should land before
   CW8.
4. Keep Option D (Windows guest, Drag D6) as the fallback for DRM, kernel drivers and
   GPU plugins. Option C (Linux guest with Wine) only if plugins are wanted before CW8.
