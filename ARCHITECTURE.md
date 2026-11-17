# SexOS architecture

This file describes SexOS as the code builds it today, and marks the parts that the
approved redesign (`docs/handoff/KERNEL_STUDY_AND_REDESIGN_2026_11_17.md`, PR #405)
is changing. Each rule below is either true of the code or labelled **planned**.
When code and this file disagree, one of them is a bug: fix the code, or fix this file
in the same PR. `spec/*.sexspec` inventories the obligations and pins this file's hash
(`scripts/check_architectural_truth.py`).

Rewritten 2026-11-18 (decision D4). The earlier text described a system-wide
determinism invariant, a kernel that never maps memory, and a slot table that the
code had outgrown.

## 0. System Model

SexOS is a **single address space operating system (SASOS)** on a capability
microkernel, for x86_64 machines with protection keys (PKU). QEMU and KVM are the
test machines until real hardware.

- **One translation.** Every core runs on the root the bootloader handed over
  (`smp.rs` asserts `SMP_SECOND_TRANSLATION_ROOT`). A virtual address names the same
  byte for every domain, for its whole life. There are no per-program address
  spaces, no fork, and no copying between address spaces.
- **Protection domains** are the units of isolation: a BOOT service, a program
  record, or a program run. A domain has an id, a capability table, a PKU key and
  a PKRU value, and one or more tasks.
- **Capabilities** decide what a domain may name in a call or a kernel operation.
- **PKU** decides what a domain's loads and stores may touch.

### Three layers

```
GLOBAL_VAS     addressing   what can be named (one translation for everyone)
capabilities   authority    what a domain may invoke or be granted (sex-pdx ABI)
PKU            enforcement  what a domain's loads and stores reach in hardware
```

1. A global address grants nothing. Holding an address is not holding access.
2. Capabilities are the only source of authority. The kernel checks them on every
   call that names another domain or a kernel object; PKU state never feeds an
   authorization decision (`scripts/pku_parity_gate.py` compares authority records
   with and without PKU).
3. PKU enforces the memory side of those decisions. **Without PKU there is no
   hardware isolation between domains today**: every domain's pages are user pages
   in one table, so a hostile program could read or write any other domain. That is
   a release blocker, not a softer mode. Planned fix (D2, approved): per-core
   *permission views*, below.

### Capability genesis

- `BOOT_DAG` (`kernel/src/init.rs`, `boot_memory.rs`) creates every domain and every
  root capability in a fixed order, then seals the tables before any task runs
  (`capability_table.rs` `seal`, `boot_memory::seal_all`).
- After the seal, capabilities can be revoked but not created. Runtime changes go
  through kernel objects that BOOT granted (program records, app lifecycle grants,
  pager mappings, device leases).
- **Planned (D1, approved 2026-11-17):** runtime minting from BOOT-held roots.
  A domain holding a root may mint a narrower capability (fewer rights, a badge)
  into a domain that holds a receive slot. Revocation bumps a generation, and the
  death of an incarnation revokes everything minted from it. Nothing can be minted
  without a root, so BOOT_DAG stays the only source of authority.

### Determinism, scoped to UiIR

Determinism is a property of **UiIR streams and the Display scene**, not of the
whole machine. Timers, interrupts, IPIs, work stealing and device completion
order make kernel state nondeterministic, and the kernel keeps no replay log.

- A UiIR stream carries a contiguous per-sender sequence and non-decreasing epochs,
  and its consumer checks both.
- Display builds one immutable scene per drain and publishes it whole
  (snapshot, transform, swap, commit).
- Full-system replay from epoch 0 is an open goal for a ring-3 recorder server.
  It is not a kernel invariant and does not constrain kernel design.

## 1. Core Architecture

### 1.1 sex-pdx (IPC and the user ABI)

The user-facing ABI is `crates/sex-pdx`. Calls name a capability slot in the
caller's own table.

| Syscall | Name | What it does |
|---|---|---|
| 0 | `PDX_CALL` | Send opcode + 3 words to the domain a slot names; take a reply credit |
| 28 | `PDX_LISTEN` | Take the next inbound call |
| 29 | `PDX_REPLY` | Answer a call the replier holds a credit for |
| 53 | `PDX_WAIT_SELF` | Wait for this domain's own inbox or reply events |
| 100 / 101 | `PDX_CALL_WAIT` / `PDX_REPLY_WAIT` | Call or reply, then wait, in one entry |

Each domain has a 256-entry inbound ring and a reply-credit table. Larger data
moves through BOOT-paired regions, lends and UiIR rings. Other syscalls are
kernel operations on capabilities the caller holds (memory, program rows, power
84, crash log 85, device 86, stats 87, Denim 88, DMA 90, VM 110, civil time 76).

**Planned (Phase 2):** endpoints with rendezvous and badges, one-shot reply
capabilities cancelled when the caller dies, notifications (a 64-bit word that IRQs
can set), 8 message registers plus a 4 KiB IPC buffer, and a same-core round trip
of at most 1,000 cycles on KVM. PR #402 adds 102 to 104 (receive in registers,
deadline scheduling) as a step towards it.

### 1.2 sext

sext is the ring-3 pager. The kernel traps a user page fault, checks that the
faulting task is live and holds a pager capability (`SLOT_SEXT`), and forwards the
fault (`kernel/src/ipc/pagefault.rs` `forward_page_fault`). A page Denim holds goes
to Denim instead.

```
#PF in ring 3
  -> kernel: live task? pager capability? (fails closed otherwise)
  -> forward to sext (or Denim for a page it holds)
  -> sext picks a frame from its BOOT-granted pool and zeroes it (zero_owner=sext)
  -> sext asks the kernel to map it through its PagerMapping capability
  -> kernel checks the grant, writes the entry, resumes the task
```

sext owns the policy and the zeroing. The kernel owns the page-table write and
the bootstrap reservations (initial backing, VA windows, table pages). Kernel
faults never enter this path, and protection, PKU, reserved-bit and
instruction-fetch faults end the program instead of paging.

### 1.3 silk-shell

silk-shell (domain 3) is the desktop's orchestrator: it starts and stops program
records (it holds the one `RecordPool` capability), routes launches and file
grants, and owns the session. It is a capability holder like any other service.

### 1.4 UiIR

UiIR is the shared-memory frame protocol between producers and Display.

- BOOT pairs each stream's ring and cursor regions between exact peers, one
  writer per page (`boot_memory.rs` `init_uiir`). The grant is a capability.
- A stream carries per-sender sequence numbers and non-decreasing epochs; the
  consumer refuses gaps and regressions.
- UiIR is a data transport, not IPC: notifications still go through sex-pdx.
- Today one stream exists (Shell to Display, focus transitions). The determinism
  rule in section 0 applies to UiIR streams and Display's scene.

### 1.5 The kernel

The kernel holds mechanism only (`docs/DESIGN_GUARDRAILS.md` section 1). Its jobs:

| Job | Where |
|---|---|
| Entry: `SYSCALL`/`SYSRET`, exceptions, interrupts, per-task kernel stacks | `interrupts.rs`, `gdt.rs` |
| Scheduling: per-core schedulers, work stealing, urgent queue, partner handoff | `scheduler*.rs`, `smp.rs` |
| IPC: calls, replies, waits, reply credits | `ipc.rs`, `ipc/*`, `ipc_ring.rs` |
| Capabilities: tables, checks, revocation | `capability*.rs`, `capabilities/` |
| Frames and mappings (see below) | `memory/*`, `boot_memory.rs`, `app_resources.rs`, `program_*.rs`, `denim.rs` |
| PKU keys and PKRU per domain | `pku.rs`, `program_tenancy.rs` |
| Interrupt routing to driver domains, device leases, IOMMU domains | `device_irq.rs`, `device_lease.rs`, `iommu.rs` |
| Drag virtual machines (VT-x, EPT) | `vmx.rs` |
| What only ring 0 can do when everything else is dead: stop screen, crash log, fixed-hardware reset and power off | `console.rs`, `crash_log.rs`, `hal/acpi.rs` |

The kernel does not run drivers, filesystems, protocol stacks or firmware
interpreters (AML runs in `servers/sexacpi`).

**The kernel maps memory.** It is the only code that writes page tables, and it
does so only on a capability-checked request or a BOOT_DAG step:

- BOOT regions and their pairings between services (`boot_memory.rs`);
- program row windows, heaps and code pages (`app_resources.rs`,
  `program_records.rs`, `program_frames.rs`, `program_image.rs`);
- which program row is present under the shared key (`program_tenancy.rs`);
- pages Denim packs and returns (`denim.rs`);
- IPC buffers and lends (`ipc/buffer.rs`), IOMMU tables, Drag EPT;
- mappings that sext asks for (section 1.2).

Policy (which page, when, for whom) lives in servers; the kernel checks the
capability and performs the write.

Entry hardening (audit: `docs/handoff/KERNEL_SECURITY_AUDIT_2026_11_18.md`): SMEP and
UMIP on where the CPU has them, enhanced or automatic IBRS against Spectre v2,
NMIs on their own IST stack, `SFMASK` clears IF, DF, TF, AC, NT and IOPL, SYSRET
never returns to a non-canonical RIP, and the kernel is built without SSE so no
kernel data reaches user vector registers. SMAP is off until user-memory access
goes through copy helpers. On syscall entry PKRU is set to 0 (all keys open) for
the length of the call.

### 1.6 SMP and scheduling

- One scheduler per core (up to 128): a work-stealing run queue, an urgent queue
  for interactive wakes, and a handoff slot that runs a call's partner next.
- Placement is a function of the domain (`smp::home_core`): desktop services on
  core 0, programs spread over spare cores, every shared-key program row on one
  core. There is no load-based migration yet.
- The kernel is not preemptible. A timer that lands in ring 0 does not switch;
  the task switches at its next return to user or wait.
- **Planned (Phase 4):** placement with load feedback and per-core locks, so the
  4-core latency gate passes with p99 under 2 ms.

### 1.7 Boot (BOOT_DAG)

Limine loads the kernel and the service modules. `init.rs` spawns BOOT services in
a fixed order, so their domain ids are stable, then the optional services
(sexacpi 50, Denim 51, Cufflinks 52) and the boot incarnations of Quil, Spindle,
Kaleidoscope and the program chain. It pairs regions, grants routes and leases,
seals every table, and starts the schedulers. No domain is created after the seal
except through program records.

### 1.8 Security model

- No ambient authority: a global address, slot 0 and ring 0 grant nothing.
  Slot 0 reaches three caller-scoped bridge operations only (BOOT display query,
  bounded serial record, own init argument; `capability_slot.rs`).
- No ring-3 code may contain `WRPKRU`, `XRSTOR` or `XRSTORS`, so it cannot open
  other keys: the program loader checks programs and code they seal
  (`sex_pdx::program_code`), the BOOT loader checks every service and refuses a
  segment that is both writable and executable (`kernel/src/elf.rs`), and the build
  relinks a binary whose layout made those bytes by chance (`scripts/key_bytes.py`).
- Organs: a PKU key belongs to a group of trusted services (section 2). Services in
  one organ are not isolated from each other in memory; capabilities still separate
  their authority.
- Known gaps, tracked in the audit: no isolation without PKU (until D2), kernel
  runs with all keys open, no SMAP, Spectre v1 hardening of table lookups, and
  Meltdown-PK on CPUs without `RDCL_NO`.

## 2. Domains, organs and keys

BOOT services, by spawn order. A domain's PKU key is its id unless it shares an
organ (`sex_pdx::organs::service_key`).

| Domain | Service | Key | Organ / role |
|---|---|---|---|
| 1 | sexdisplay | 1 | Core: compositor and scanout |
| 2 | sexdrive | 2 | Storage devices (NVMe, AHCI) |
| 3 | silk-shell | 3 | Desktop orchestration |
| 4 | sexinput | 4 | Senses: input |
| 5 | sexusb | 4 | Senses: USB host |
| 6 | silkbar | 1 | Core: panel, shares Display's key |
| 7 | linen | 7 | Linen file manager surface |
| 8 | sexstore | 11 | Storage: key/value store, shares Files' key |
| 9 | quil | 9 | Quil editor surface |
| 10 | sexbell | 4 | Senses: sound and attention |
| 11 | sexfiles | 11 | Storage: files and the program store |
| 12 | spindle | 12 | Spindle terminal |
| 13 | sexnet | 13 | Network |
| 14 | sext | 14 | Memory: pager |
| 15 | kaleidoscope | 15 | Browser |
| 50 | sexacpi | 4 | Senses: ACPI AML in ring 3 |
| 51 | Denim | 14 | Memory: compressed pages |
| 52 | Cufflinks | 4 | Senses: Bluetooth |

Programs: rows 0 and 1 run on key 5, row 2 on key 6, and every later row on key 8,
where rows take turns being present (one core, a local TLB flush per turn).
Program records hold key 10. Key 0 is the kernel's.

**Planned (D2, approved):** per-core PCID-tagged permission views. A view is a
private copy of the upper-level tables whose user bits differ, sharing every
page-table page below that. Each shared-key row gets its own view, so rows run on
any core without turn-taking or full flushes, and without PKU each organ gets a
view, which gives isolation on machines without PKU. Views change permissions
only; the translation stays one.

## 3. Capability slot conventions

A domain's table has 1,024 slots. Slot numbers are conventions: the same number
means the same kind of route in every domain that holds it. Slot 0 is the kernel
bridge (section 1.8). The main fixed slots (`crates/sex-pdx/src/lib.rs`):

| Slot | Holds |
|---|---|
| 1 | Files (sexfiles) |
| 2 | Pager (sext) |
| 3 | Input |
| 4 | Audio |
| 5 | Display |
| 6 | Shell |
| 7 | SilkBar |
| 8 | USB host controller lease |
| 10 | sexstore |
| 11 / 13 / 14 | Quil / Linen / Spindle surfaces |
| 12 | Bell |
| 15 | Block service (sexdrive) |
| 16 | First NVMe controller lease |
| 17 | Block buffer lend |
| 18 / 19 | Network route / NIC lease |
| 20-25 | Sound, AHCI, Wi-Fi (device, DMA, firmware), GPU leases |
| 32-37 | DMA region, frame region, frame pool, demand region, pager mappings |

Capability kinds are listed in `kernel/src/capability.rs` `CapabilityData`.
**Planned (Phase 3):** inline 32-byte slots (object, generation, rights, badge),
new Endpoint, Notification and Reply kinds, and removal of the unused kinds.

## 4. Hard constraints

- One translation. No per-program address spaces, no Linux-style processes.
  Permission views (D2) share the translation and are not address spaces.
- No ambient authority: every access decision comes from a capability.
- No drivers, filesystems, protocol stacks or firmware interpreters in the kernel;
  no Linux-shaped syscalls (fork, pipes, fds, sockets). Compatibility layers are
  libraries; Linux drivers are temporary ring-3 bridges with a tracked native
  replacement (`docs/DESIGN_GUARDRAILS.md`, `docs/BRIDGE_LEDGER.md`).
- The kernel stays under its line budget (22,700 non-blank lines,
  `scripts/tests/test_kernel_design_guardrails.py`).
- Programs never get `WRPKRU`/`XRSTOR`; XCR0 never includes PKRU.
- UiIR streams stay ordered per sender with non-decreasing epochs.

## 5. Repository layout

```
kernel/          ring-0 mechanism (section 1.5)
crates/sex-pdx/  user-facing ABI: syscalls, slots, organs, grants
crates/          shared libraries (sex-rt, sex-std, sex-libc, drivers, codecs...)
servers/         BOOT services (sexdisplay, sexfiles, sext, silk-shell, ...)
apps/            programs (Quil, Kaleidoscope, Spreadeagle, Moan, ...)
spec/            obligation inventory pinned to this file
docs/            design rules, handoffs, reports
```

## 6. Build authority

Builds go through one path:

- `sexos_build_spec.toml`: the declarative build specification;
- `scripts/entrypoint_build.sh`: the only build root;
- `scripts/sexos_build_trace.sh`: the interpreter of the build spec.

Direct routes (`build_payload.sh`, a bare `make iso` or `run-sasos`) are invalid
and must fail. Full builds and QEMU gates run on GitHub CI from a pushed branch.
