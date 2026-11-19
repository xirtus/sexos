# Kernel security audit: entry, exit and CPU hardening (2026-11-18)

Phase 1c of the kernel redesign (`docs/handoff/KERNEL_STUDY_AND_REDESIGN_2026_11_17.md`,
finding F8). Scope: the ring transitions (`kernel/src/interrupts.rs`), descriptor
tables (`kernel/src/gdt.rs`), CPU feature setup (`kernel/src/hal/x86_64.rs`), and the
CPU-level questions the plan raised: SYSRET with a non-canonical RCX, SMAP/SMEP/UMIP,
Spectre v1/v2, Meltdown and Meltdown-PK, and leaked kernel stacks.

Fixes were allowed only in those three files (+60 lines). One more fix, building the
kernel without SSE, had to touch the kernel target file `kernel/x86_64-sex.json`,
which no stream owns. Everything else is reported here with an owner.

Kernel growth in this PR: +54 non-blank lines (interrupts.rs +19, gdt.rs +8,
hal/x86_64.rs +27).

## Summary

| # | Finding | Severity | Status |
|---|---|---|---|
| S1 | A program can stop the machine by setting RFLAGS.NT before a blocking syscall | High | Fixed |
| S2 | NMIs run on the caller's stack in the SYSCALL and SYSRET windows | High | Fixed |
| S3 | Interrupts can be enabled between `pop rsp` and `sysretq` | High | Fixed |
| S4 | The kernel uses SSE registers, so user XMM state is clobbered and kernel data leaks into it | High | Fixed |
| S5 | SYSRET to a non-canonical RCX faults in ring 0 (CVE-2012-0217 class) | Medium | Fixed |
| S6 | SMEP and UMIP are off | Medium | Fixed (where the CPU has them) |
| S7 | Spectre v2: no IBRS of any kind | Medium | Partly fixed (eIBRS / AutoIBRS) |
| S8 | Conditional SWAPGS can be speculated past (CVE-2019-1125) | Low | Fixed |
| S9 | A program can exhaust kernel memory by churning worker threads | Medium | Open: `program_workers.rs` owner (WS1) |
| S10 | Spectre v1 gadgets on capability-slot lookups | Medium | Open: stream 3 (new table) |
| S11 | SMAP off and PKRU = 0 for the whole syscall | Medium | Open: Phase 6 design |
| S12 | Meltdown and Meltdown-PK on CPUs without `RDCL_NO` | Info | Reported at boot |
| S13 | Unused user-accessible per-core page on key 15 | Low | Open: `core_local.rs` owner (WS4) |
| S14 | Cross-domain branch-target injection between programs | Info | Open: decision for Phase 5/6 |
| S15 | BOOT services and bridges were never scanned for WRPKRU/XRSTOR/XRSTORS | High | Fixed |
| S16 | Three kernel user mappings were writable and executable; program records' code was never scanned | High | Fixed |
| S17 | The framebuffer grant sets the user bit on whole huge pages | Medium | Open: `init.rs`/boot display owner |

Checked and fine: syscall and interrupt returns restore every general register from
the saved user frame (no kernel register contents reach ring 3); the GDT has no
32-bit code segment, so programs cannot enter compatibility mode; the TSS has no I/O
permission bitmap and user IOPL is always 0, so ring 3 has no port I/O; kernel stacks
for program records come from a fixed pool.

## Findings

### S1. RFLAGS.NT turns a blocking syscall into a kernel #GP (fixed)

`POPF` lets ring 3 set NT. `SFMASK` cleared IF, DF, TF and AC but not NT, so a
syscall ran with the caller's NT. When that syscall blocks (listen, wait, yield),
the scheduler switches tasks with `iretq` (`scheduler.rs` `switch_to`). `IRET` with
NT=1 in long mode raises #GP, in ring 0, which is a kernel panic and the stop
screen. Any program could do it with three instructions.

Fix: `SFMASK` now also clears NT and IOPL (Linux masks the same set). SYSRET
restores the caller's own flags from R11, so programs see no change.

### S2. NMIs on the caller's stack (fixed)

`SYSCALL` does not switch stacks. Between `syscall` and `mov rsp, gs:[16]`, and
again between `pop rsp` and `sysretq`, RSP holds the caller's value while CPL is 0.
The NMI gate had no IST entry, so an NMI there pushed its frame at the
caller-chosen RSP, in ring 0. A program that sets RSP to a kernel address before
`syscall` gets the CPU to write an interrupt frame there (Linux moved NMI, #DB and
#MC to IST stacks for this reason). Without SMAP it also lets another core's thread
of the same program edit the frame.

Fix: NMIs use IST 3 on every core (16 KiB, static on the BSP, allocated per AP in
`gdt::init_core`). The NMI handler touches no GS-relative data. #DB cannot be
raised from these windows (TF is masked, programs cannot set debug registers) and
#MC has no handler (CR4.MCE is not set), so neither needs IST today. If a #MC
handler is added, it needs IST too.

### S3. Interrupts enabled at SYSRET (fixed)

A syscall that waits goes through `hal::acpi::idle` (`sti; hlt` or `sti; mwait`)
and can come back with IF set. The return path then restored the user RSP and
user GS before `sysretq`. A timer or device interrupt in that gap would push its
frame onto the caller's stack in ring 0; the stub sees CS.RPL=0, skips SWAPGS, and
the Rust handler reads `CoreLocal` through the user GS base.

Fix: `cli` right after `syscall_handler` returns. SYSRET loads the caller's IF from
R11, so programs still run with interrupts on.

### S4. Kernel SSE use (fixed)

`kernel/x86_64-sex.json` had `"-mmx,-sse,+sse2"`, and `+sse2` turns SSE back on.
The kernel binary had 6,517 XMM instructions, in syscall paths too (`iommu::call`,
formatting, copies). The FPU state is saved only on task switches, so:

- every syscall could change a program's XMM registers, and the `sex-pdx` syscall
  wrappers do not declare them clobbered, so SSE code in programs could see its
  values change across a call;
- bytes the kernel copied through XMM registers (another domain's data included)
  stayed in the caller's registers after SYSRET.

Fix: the kernel target is now `"-mmx,-sse,-sse2,+soft-float"` with
`"rustc-abi": "softfloat"`, the same as the root target. The rebuilt kernel has no
XMM instructions. Stream 5 (XSAVE/AVX) is unaffected: it saves user state, and the
kernel no longer touches it.

### S5. SYSRET with a non-canonical RCX (fixed)

On Intel, `sysretq` with a non-canonical RCX raises #GP in ring 0 after RSP is
already the caller's. RCX is the address after the caller's `syscall`, so it is
non-canonical only if a program runs `syscall` as the last two bytes below the
canonical hole (0x7FFF_FFFF_FFFE). Nothing in the code proves that page can never
be mapped executable.

Fix: before restoring registers, the return path checks bits 63:47 of the saved
RIP. If any is set, it returns to address 0 instead, where the fetch faults in
ring 3 and ends only that task. Three instructions on the common path.

### S6. SMEP and UMIP (fixed where present)

CR4 had neither. Without SMEP, a kernel bug that jumps to a user address runs
program code in ring 0. Without UMIP, any program can read the GDT, IDT and TSS
addresses with `sgdt`/`sidt`/`str`.

Fix: `hal::x86_64::harden_core` sets CR4.SMEP and CR4.UMIP when CPUID leaf 7
reports them. The BSP sets them before `smp::boot_aps` copies CR4 to the other
cores, and each AP calls it again from `gdt::init_core`. QEMU `-cpu max` (all CI
gates) and KVM `-cpu host` report both, so the gates exercise them. A program that
uses `sgdt` now takes #GP and ends; Drag VMs run guest code in VMX non-root and are
not affected.

SMAP stays off (see S11).

### S7. Spectre v2 (partly fixed)

Nothing restricted indirect-branch prediction. Now, per core:

- Intel with `IA32_ARCH_CAPABILITIES.IBRS_ALL`: `IA32_SPEC_CTRL.IBRS` set once
  (enhanced IBRS);
- AMD with AutoIBRS (CPUID 0x8000_0021 EAX bit 8): `EFER.AIBRSE` set (copied to APs
  with EFER).

The boot log prints `[cpu.hardening] smep= smap=0 umip= ibrs= rdcl_no= meltdown_pk=`.
CPUs with neither feature, which includes the EPYC 7763 (Zen 3) behind most GitHub
runners, report `ibrs=none` and stay unmitigated against ring 3 to ring 0 BTI.
The remaining options are retpolines (a rustc flag in the kernel build) or IBRS on
every entry; both cost cycles on the hot path and belong with the Phase 2
fastpath measurements on KVM.

### S8. SWAPGS speculation (fixed)

Each exception and interrupt stub runs `swapgs` only when it came from ring 3.
The CPU can speculate down the wrong side and issue GS-relative loads with the
wrong base. Fix: `lfence` where both paths join, in the four stubs that swap.

### S9. Worker churn exhausts kernel memory (open, WS1)

`program_workers::spawn` reuses a dead worker task only after it has been dead for
4 ticks; otherwise it leaks a new `Task` with a 64 KiB kernel stack. The limit
counts live workers only. A program that starts workers which exit at once, in a
loop, allocates a new kernel stack per iteration until the kernel heap or frames
run out. Suggested fix in `program_workers.rs`: also cap the row's total tasks
(live plus dead, e.g. `2 * MAX_WORKERS`) and return `FULL` until one is reusable.
Program-record tasks are a fixed pool and are fine.

### S10. Spectre v1 on slot lookups (open, stream 3)

`decode_capability_slot` bounds-checks a register value and the table is then
indexed with it. Speculatively, the index can run past the check. Stream 3's new
inline-slot table should clamp the index without a branch
(`index & (SLOTS - 1)` with a power-of-two table, or a `cmov`-based mask) before
the load. The same applies to `decode_domain_id` users (`DOMAIN_REGISTRY`).

### S11. SMAP off, PKRU = 0 in the kernel (open, Phase 6)

The kernel reads and writes user memory directly in many places, so SMAP cannot be
turned on without wrapping every user access in `stac`/`clac`. And `syscall_entry`
opens every PKU key for the whole call. Together, any kernel bug that follows a
caller's pointer can reach every domain's memory. The plan's Phase 6 user gates
should come with: one user-copy module (the only place `stac`/`clac` appear), SMAP
on, and PKRU set to the caller's keys plus the kernel's instead of 0.
`SFMASK` already clears AC, so SMAP can be enabled without entry changes.

### S12. Meltdown and Meltdown-PK (reported)

All domains and the kernel share one translation, so a CPU vulnerable to Meltdown
lets ring 3 read kernel memory transiently, and one vulnerable to Meltdown-PK lets
a program read pages its PKRU denies. Per-mode page tables (KPTI) would undo the
single address space. `RDCL_NO` in `IA32_ARCH_CAPABILITIES` marks CPUs that are
not affected; the boot marker shows it (`meltdown_pk=no` or `possible`). Intel
Ice Lake and later (the x17 R1's Tiger Lake included) and AMD parts report or are
not affected; Skylake-SP is the PKU-capable part that is. Recommendation: treat
`meltdown_pk=possible` as "PKU isolation is integrity only" in release notes.

MDS-class leaks (`MDS_NO` clear) would need `verw` before returning to ring 3. The
target hardware sets `MDS_NO`, so this is not added; revisit if older CPUs matter.

### S13. Per-core syscall message page (open, WS4)

`CoreLocal::init_with_tss` maps a 4 KiB page per core, user-accessible, on key 15,
and stores it in `syscall_msg_ptr`, which nothing reads. Key 15 is Kaleidoscope's.
The page carries no data today, but it is shared writable memory between every
domain that can open key 15. Remove it in `core_local.rs`.

### S14. Cross-domain branch-target injection (decision needed)

Domains switch without a CR3 write and without IBPB, so one program can train
indirect branches that another program (or a service on the same core) then
mispredicts. Enhanced IBRS protects the kernel, not one user domain from another.
Options: IBPB when switching between domains of different trust (thousands of
cycles per switch on most parts), or STIBP plus core placement. This belongs with
the Phase 5 placement and Phase 6 isolation work.

### S15. BOOT services and bridges could hold key instructions (fixed)

Programs have been checked for WRPKRU, XRSTOR and XRSTORS since the program loader
was written, at load and when they seal JIT code. BOOT services, including driver
servers that run Tuxedo bridges, went through the older BOOT loader
(`kernel/src/elf.rs`), which checked nothing. A service binary holding `0F 01 EF`
anywhere in its code (a compiler can make it by chance) gives anyone who can steer
that service's control flow every key.

Fix:
- the BOOT loader refuses a module with a writable and executable segment, and
  scans each code segment's final, relocated bytes with the same
  `sex_pdx::program_code::Scan` the program loader uses; a hit logs
  `[boot.service.rejected]` and the service does not start;
- the build trace runs `scripts/key_bytes.py` on every Cargo-built boot module, not
  only programs, and relinks a hit with shuffled sections (services get the
  shuffle as link arguments, programs keep `SEX_LAYOUT_SEED`).

Today's services were built and scanned before the refusal was turned on: no hits
and no writable-and-executable segments in the 16 that build on their own (sexfiles,
sexnet, Kaleidoscope and Cufflinks need the full build trace and are checked by CI).
The byte check holds only while code cannot change: services have no
writable-and-executable memory, and only programs can turn pages into code
(`SYS_PROGRAM_CODE`, which scans).

### S16. W^X for every user mapping, and record code (fixed)

The byte scan holds only if no ring-3 code can write a page and then run it. Every
path that maps user memory was checked:

| Path | Before | Now |
|---|---|---|
| Program images, heaps, stacks (`app_resources.rs`) | W^X, loader refuses W+X segments | unchanged |
| Program records (`program_records.rs`) | W^X, but code **not scanned** | scanned like programs (`[record.rejected]`) |
| BOOT services (`elf.rs`) | per-segment flags, W+X allowed | W+X refused, code scanned (S15) |
| BOOT regions, pager mappings, Denim, app aliases | NX on every writable page | unchanged |
| `pku::set_page_user_accessible` (framebuffer grant) | set U+W, **left NX as it was** | sets NX |
| Per-core syscall page (`core_local.rs`, S13) | U+W, **executable**, key 15 | removed on master by WS4 |
| IPC buffers (`ipc/buffer.rs`) | U+W, **executable**, key 14 | NX |
| `SYS_PROGRAM_CODE` seal (JIT) | scans before making pages code | unchanged |

After this, a page a ring-3 domain can write is never executable, and every page it
can execute was scanned once its bytes were final. The cross-domain isolation gate
(stream 7) should include "write a page, jump to it, expect a fault".

### S17. Framebuffer grant on huge pages (open)

`init.rs` gives Display the framebuffer by calling `set_page_user_accessible` per
4 KiB, but when the bootloader mapped it with a 2 MiB or 1 GiB page, the function
sets the user bit on the whole huge page. Display then reaches everything else in
that page, which can include other devices' MMIO. Fix: map the framebuffer into a
BOOT region window with 4 KiB pages (as other regions are) instead of changing the
HHDM entry.

## Kernel stacks

- `Task::with_kernel_stack` leaks its stack `Vec` on purpose: tasks are never freed.
- Program records allocate their tasks once per record slot (fixed pool).
- Program workers reuse dead tasks per row, but see S9 for churn.
- Each core leaks a 32 KiB double-fault stack and a 16 KiB NMI stack once, at start.
- `init.rs` `jump_to_userland` is dead code and builds an RFLAGS with IOPL=3; delete
  it with the Phase 1a cleanup.

## Verification

- The kernel builds with the new target; `objdump` shows no XMM instructions and
  the syscall exit path as intended (`cli`, RIP check, then restores and `sysretq`).
- Host checks: kernel design guardrails (line budget), architectural truth
  inventory, and the full `scripts/tests` suite.
- The QEMU gates on CI run with `-cpu max`, which reports SMEP and UMIP, so every
  gate boots with both on. The eIBRS and AutoIBRS paths are not exercised by TCG;
  they need a KVM run on a CPU that has them.
